import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';

type ControlPlaneTransport = Pick<DesktopRuntimeGateway, 'sendControlPlaneRequest'>;

const nullBodyStatuses = new Set([204, 205, 304]);

/**
 * 桌面端发往控制面的请求交给原生层代发，原生层带上登录。macOS 的 WKWebView 不替跨站请求带
 * Cookie：直接用 fetch 请求控制面，浏览器里登录完回来，界面还是“没登录”。别的地址照常走 fetch。
 *
 * 和 fetch 一样：连不上时抛 `TypeError`，取消时抛 signal 的 reason。
 */
export function desktopControlPlaneFetch(
  baseUrl: string,
  transport: ControlPlaneTransport,
  fallback: typeof globalThis.fetch = globalThis.fetch.bind(globalThis),
): typeof globalThis.fetch {
  const base = new URL(baseUrl);
  const prefix = base.pathname.endsWith('/') ? base.pathname : `${base.pathname}/`;
  return async (input, init) => {
    const url = new URL(requestUrl(input));
    if (url.origin !== base.origin || !url.pathname.startsWith(prefix)) {
      return await fallback(input, init);
    }
    const signal = init?.signal ?? (input instanceof Request ? input.signal : null);
    throwIfAborted(signal);
    const method = (
      init?.method ?? (input instanceof Request ? input.method : 'GET')
    ).toUpperCase();
    const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : {}));
    const body = await requestBody(input, init, method, headers);
    throwIfAborted(signal);
    const sent = await untilAborted(
      transport.sendControlPlaneRequest({
        body,
        headers: [...headers],
        method,
        path: `${url.pathname.slice(prefix.length)}${url.search}`,
      }),
      signal,
    );
    if (!sent.ok) {
      throw new TypeError(`控制面请求没能发出：${sent.error.code}`);
    }
    const { body: responseBody, headers: responseHeaders, status } = sent.value;
    return new Response(method === 'HEAD' || nullBodyStatuses.has(status) ? null : responseBody, {
      headers: responseHeaders.map(([name, value]): [string, string] => [name, value]),
      status,
    });
  };
}

function requestUrl(input: Parameters<typeof fetch>[0]): string {
  if (typeof input === 'string') return input;
  return input instanceof URL ? input.href : input.url;
}

/**
 * 正文统一转成字节。FormData、URLSearchParams、Blob 自带的类型（含 multipart 边界）在请求头里
 * 没写时补上，和浏览器自己发时一样。
 */
async function requestBody(
  input: Parameters<typeof fetch>[0],
  init: RequestInit | undefined,
  method: string,
  headers: Headers,
): Promise<Uint8Array<ArrayBuffer>> {
  if (method === 'GET' || method === 'HEAD') return new Uint8Array();
  const source =
    init?.body !== undefined
      ? new Response(init.body)
      : input instanceof Request
        ? input.clone()
        : null;
  if (source === null) return new Uint8Array();
  const contentType = source.headers.get('content-type');
  if (contentType !== null && !headers.has('content-type')) {
    headers.set('content-type', contentType);
  }
  return new Uint8Array(await source.arrayBuffer());
}

function throwIfAborted(signal: AbortSignal | null): void {
  if (signal?.aborted === true) {
    throw abortReason(signal);
  }
}

/** 原生请求停不下来；取消时先放手，结果到了也不再用。 */
async function untilAborted<TValue>(
  pending: Promise<TValue>,
  signal: AbortSignal | null,
): Promise<TValue> {
  if (signal === null) return await pending;
  // abort 事件已经过去的话，再挂监听等不到它。
  throwIfAborted(signal);
  return await new Promise<TValue>((resolve, reject) => {
    const abort = () => {
      reject(abortReason(signal));
    };
    signal.addEventListener('abort', abort, { once: true });
    pending.then(
      (value) => {
        signal.removeEventListener('abort', abort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener('abort', abort);
        reject(error instanceof Error ? error : new TypeError('控制面请求没能发出'));
      },
    );
  });
}

function abortReason(signal: AbortSignal): Error {
  const reason: unknown = signal.reason;
  return reason instanceof Error
    ? reason
    : new DOMException('The operation was aborted.', 'AbortError');
}
