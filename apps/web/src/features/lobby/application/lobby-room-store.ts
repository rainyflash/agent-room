import type { LobbyGateway, LobbyRoom } from '@/features/lobby/domain/lobby';

export type LobbyRoomState =
  | { readonly kind: 'loading' }
  | {
      readonly code:
        'lobby.matrix_unavailable' | 'lobby.room_not_joined' | 'lobby.room_projection_invalid';
      readonly kind: 'failed';
      readonly retryable: boolean;
    }
  | { readonly kind: 'ready'; readonly room: LobbyRoom };

const loadingState: LobbyRoomState = Object.freeze({ kind: 'loading' });

export class LobbyRoomStore {
  readonly #gateway: LobbyGateway;
  readonly #listeners = new Set<() => void>();
  readonly #roomId: string;
  #detachGateway: (() => void) | null = null;
  #state: LobbyRoomState = loadingState;

  constructor(gateway: LobbyGateway, roomId: string) {
    this.#gateway = gateway;
    this.#roomId = roomId;
  }

  readonly getSnapshot = (): LobbyRoomState => {
    return this.#state;
  };

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
