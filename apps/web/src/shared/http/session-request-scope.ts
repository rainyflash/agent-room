/** Cancels private HTTP work when its owning account state is cleared. */
export class SessionRequestScope {
  #controller = new AbortController();
  readonly fetch: typeof globalThis.fetch;

  constructor(fetchImplementation: typeof globalThis.fetch = globalThis.fetch.bind(globalThis)) {
    this.fetch = (input, init) => {
      const requestSignal =
        init?.signal !== undefined
          ? init.signal
          : input instanceof Request
            ? input.signal
            : undefined;
      if (requestSignal === undefined || requestSignal === null) {
        return fetchImplementation(input, { ...init, signal: this.#controller.signal });
      }
      const combined = eitherSignal(this.#controller.signal, requestSignal);
      return fetchImplementation(input, { ...init, signal: combined.signal }).finally(
        combined.release,
      );
    };
  }

  clear(): void {
    const previous = this.#controller;
    this.#controller = new AbortController();
    // Cancel before revoking login; clearing cached results alone leaves HTTP requests alive.
    // This does not roll back writes that the server has already committed.
    previous.abort(new DOMException('The owning session was cleared.', 'AbortError'));
  }
}

type CombinedSignal = { readonly signal: AbortSignal; readonly release: () => void };

// AbortSignal.any 要 iOS/Safari 17.4、Chrome 116 起才有。旧手机上缺了它，
// 每个带超时的请求都会在发出前抛错，界面只会说“连不上”。
function eitherSignal(first: AbortSignal, second: AbortSignal): CombinedSignal {
  if (typeof AbortSignal.any === 'function') {
    return { signal: AbortSignal.any([first, second]), release: () => undefined };
  }
  const combined = new AbortController();
  // 请求结束就摘掉监听，免得会话级信号上越挂越多。
  const listening = new AbortController();
  const release = () => {
    listening.abort();
  };
  for (const source of [first, second]) {
    if (source.aborted) {
      combined.abort(source.reason);
      release();
      break;
    }
    source.addEventListener(
      'abort',
      () => {
        combined.abort(source.reason);
        release();
      },
      { once: true, signal: listening.signal },
    );
  }
  return { signal: combined.signal, release };
}
