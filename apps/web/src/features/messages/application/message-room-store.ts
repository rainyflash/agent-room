import type { MessageGateway, MessageRoomProjection } from '@/features/messages/domain/message';

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
      this.#refresh();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detachGateway?.();
        this.#detachGateway = null;
      }
    };
  };

  readonly #refresh = (): void => {
    const result = this.#gateway.read(this.#roomId);
    this.#state = result.ok
      ? Object.freeze({ kind: 'ready', room: result.value })
      : Object.freeze({
          code: result.error.code,
          kind: 'failed',
          retryable: result.error.retryable,
        });
    for (const listener of this.#listeners) {
      listener();
    }
  };
}
