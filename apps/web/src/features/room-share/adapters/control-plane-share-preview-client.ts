import {
  SHARE_CODE_PATTERN,
  type SharePreviewGateway,
} from '@/features/room-share/domain/room-share';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';

export type ControlPlaneSharePreviewClientOptions = {
  readonly baseUrl: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly timeoutMs?: number;
};

/** 控制面画的图约一两百 KB；比这大的不当它是预览图。 */
const MAX_PREVIEW_BYTES = 4 * 1024 * 1024;

/**
 * 分享对话框里“发出去长这样”的那张图（`specs/redesign/project-rooms.md` 3P-8b-2）：经组合根注入的 fetch 去
 * 控制面要 `/r/<分享码>/preview.png` 的字节，界面拿它做成 `blob:` 地址。图在裸域上，网页和桌面端的内容安全
 * 策略都不许直接引那里的图；控制面在接口的地址上也出这条路（网页同源的 `/_agent-room/api`，桌面端由原生层
 * 代发），所以不用放宽策略。
 *
 * - 不带 `?v=`：服务器不看它，照此刻的房间名和几个数画。`cache: 'no-cache'` 让浏览器每次拿 `ETag` 去问，
 *   没变就是 304、用浏览器里那份，变了拿新的；桌面端原生层不缓存，每次都问，服务器内存里留着 10 分钟。
 * - 不跟跳转：服务器忙、画不了时 302 去通用图，这里当作没有，界面放自己带的那张通用图。
 * - 图不要登录，也不放进登录范围：发现页上没登录的人也能分享。
 */
export class ControlPlaneSharePreviewClient implements SharePreviewGateway {
  readonly #baseUrl: string;
  readonly #fetch: typeof globalThis.fetch;
  readonly #timeoutMs: number;

  constructor({
    baseUrl,
    fetch: fetchImplementation = globalThis.fetch.bind(globalThis),
    timeoutMs = 8_000,
  }: ControlPlaneSharePreviewClientOptions) {
    this.#baseUrl = baseUrl;
    this.#fetch = fetchImplementation;
    this.#timeoutMs = timeoutMs;
  }

  async roomImage(shareCode: string, signal: AbortSignal): Promise<Blob | null> {
    if (!SHARE_CODE_PATTERN.test(shareCode) || signal.aborted) return null;
    const controller = new AbortController();
    const stop = () => {
      controller.abort();
    };
    signal.addEventListener('abort', stop, { once: true });
    const timeout = globalThis.setTimeout(stop, this.#timeoutMs);
    try {
      const response = await this.#fetch(
        controlPlaneEndpoint(this.#baseUrl, `/r/${shareCode}/preview.png`),
        {
          cache: 'no-cache',
          credentials: 'omit',
          headers: { Accept: 'image/png' },
          method: 'GET',
          redirect: 'manual',
          signal: controller.signal,
        },
      );
      if (response.status !== 200 || !isPng(response.headers.get('content-type'))) return null;
      const image = await response.blob();
      if (controller.signal.aborted || image.size === 0 || image.size > MAX_PREVIEW_BYTES) {
        return null;
      }
      return image.type === 'image/png' ? image : new Blob([image], { type: 'image/png' });
    } catch {
      return null;
    } finally {
      globalThis.clearTimeout(timeout);
      signal.removeEventListener('abort', stop);
    }
  }
}

function isPng(contentType: string | null): boolean {
  return contentType?.split(';')[0]?.trim().toLowerCase() === 'image/png';
}
