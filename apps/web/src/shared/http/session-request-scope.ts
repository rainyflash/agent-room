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
      const signal =
        requestSignal === undefined || requestSignal === null
          ? this.#controller.signal
          : AbortSignal.any([this.#controller.signal, requestSignal]);
      return fetchImplementation(input, { ...init, signal });
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
