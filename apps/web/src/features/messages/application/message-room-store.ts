import type {
  MessageGateway,
  MessageReadResult,
  MessageRoomProjection,
} from '@/features/messages/domain/message';

export type MessageRoomState =
  | { readonly kind: 'loading' }
  | {
      readonly code:
        'messages.matrix_unavailable' | 'messages.room_not_joined' | 'messages.projection_invalid';
      readonly kind: 'failed';
      readonly retryable: boolean;
    }
  | { readonly kind: 'ready'; readonly room: MessageRoomProjection };

const loadingState: MessageRoomState = Object.freeze({ kind: 'loading' });

export class MessageRoomStore {
  readonly #gateway: MessageGateway;
  readonly #listeners = new Set<() => void>();
  readonly #roomId: string;
  #detachGateway: (() => void) | null = null;
  #state: MessageRoomState = loadingState;

  constructor(gateway: MessageGateway, roomId: string) {
    this.#gateway = gateway;
    this.#roomId = roomId;
  }

  get roomId(): string {
    return this.#roomId;
  }

  readonly getSnapshot = (): MessageRoomState => this.#state;

  readonly retry = (): void => {
    this.#refresh();
  };

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detachGateway = this.#gateway.subscribe(this.#roomId, this.#refresh);
      // 第一个观察者总会收到一次通知，组合它的 store 靠这次通知算出自己的状态。
      this.#update(true);
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detachGateway?.();
        this.#detachGateway = null;
      }
    };
  };

  /** 每次同步都会调用；房间没变时网关交回同一份投影，这时不通知，整段对话也就不用重新渲染。 */
  readonly #refresh = (): void => {
    this.#update(false);
  };

  #update(notifyUnchanged: boolean): void {
    const next = nextState(this.#state, this.#gateway.read(this.#roomId));
    if (next === this.#state && !notifyUnchanged) return;
    this.#state = next;
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

function nextState(current: MessageRoomState, result: MessageReadResult): MessageRoomState {
  if (result.ok) {
    return current.kind === 'ready' && current.room === result.value
      ? current
      : Object.freeze({ kind: 'ready', room: result.value });
  }
  return current.kind === 'failed' &&
    current.code === result.error.code &&
    current.retryable === result.error.retryable
    ? current
    : Object.freeze({
        code: result.error.code,
        kind: 'failed',
        retryable: result.error.retryable,
      });
}
