/**
 * 这台设备的自动签名（`specs/device-signing/design.md`）走到哪一步了。界面只读它：正在准备、
 * 已就绪、出错可重试。出错时 `retry` 重新跑一遍。
 */
export type DeviceSigningState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'working' }
  | { readonly kind: 'ready' }
  | { readonly kind: 'failed' };

export class DeviceSigningStatus {
  #state: DeviceSigningState = { kind: 'idle' };
  #retry: (() => void) | null = null;
  readonly #listeners = new Set<() => void>();

  readonly getSnapshot = (): DeviceSigningState => this.#state;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };

  /** 当前这台设备怎么重新跑一遍；换了会话或退出时清掉。 */
  setRetry(retry: (() => void) | null): void {
    this.#retry = retry;
  }

  retry(): void {
    this.#retry?.();
  }

  set(state: DeviceSigningState): void {
    this.#state = state;
    for (const listener of this.#listeners) listener();
  }
}
