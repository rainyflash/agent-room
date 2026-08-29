import type {
  DesktopLobbySnapshot,
  DesktopRuntimeFailure,
  DesktopRuntimeGateway,
} from '@/features/desktop/domain/desktop-runtime';

const POLL_INTERVAL_MS = 4_000;

export type DesktopLobbyState =
  | { readonly kind: 'loading' }
  | { readonly failure: DesktopRuntimeFailure; readonly kind: 'failed' }
  | { readonly kind: 'ready'; readonly snapshot: DesktopLobbySnapshot };

const loadingState: DesktopLobbyState = Object.freeze({ kind: 'loading' });

/** 维护桌面大厅投影的轮询生命周期，React 只负责订阅。 */
export class DesktopLobbyStore {
  readonly #gateway: DesktopRuntimeGateway;
  readonly #listeners = new Set<() => void>();
  #inFlight = false;
  #poller: ReturnType<typeof setInterval> | null = null;
  #state: DesktopLobbyState = loadingState;

  constructor(gateway: DesktopRuntimeGateway) {
    this.#gateway = gateway;
  }

  readonly getSnapshot = (): DesktopLobbyState => this.#state;

  readonly retry = (): void => {
    void this.#refresh();
  };

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      void this.#refresh();
      this.#poller = setInterval(() => {
        void this.#refresh();
      }, POLL_INTERVAL_MS);
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0 && this.#poller !== null) {
        clearInterval(this.#poller);
        this.#poller = null;
      }
    };
  };

  async #refresh(): Promise<void> {
    if (this.#inFlight) return;
    this.#inFlight = true;
    const result = await this.#gateway.readLobby();
    this.#inFlight = false;
    this.#state = result.ok
      ? Object.freeze({ kind: 'ready', snapshot: result.value })
      : Object.freeze({ failure: result.error, kind: 'failed' });
    for (const listener of this.#listeners) listener();
  }
}
