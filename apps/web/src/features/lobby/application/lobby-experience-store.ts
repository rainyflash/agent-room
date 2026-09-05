import type { LobbyGateway, LobbyRoom } from '../domain/lobby';
import type { MessageGateway } from '@/features/messages/domain/message';
import { MessageRoomStore } from '@/features/messages/application/message-room-store';
import { LobbyRoomStore, type LobbyRoomState } from './lobby-room-store';
import { projectLobbyScene, type LobbySceneProjection } from '../domain/scene-projection';
import { roomHumans, type RoomIdentity } from '../domain/room-participants';
import type { RoomLayout } from '../domain/room-layout';

type LobbyExperienceState =
  | Exclude<LobbyRoomState, { readonly kind: 'ready' }>
  | {
      readonly kind: 'ready';
      readonly room: LobbyRoom;
      readonly projection: LobbySceneProjection;
    };

/** 组合权威房间与消息订阅，位置只属于本次房间的显示状态。 */
export class LobbyExperienceStore {
  readonly messages: MessageRoomStore;
  readonly #lobby: LobbyRoomStore;
  readonly #identity: RoomIdentity | null;
  readonly #listeners = new Set<() => void>();
  #detach: readonly (() => void)[] = [];
  #layout: RoomLayout = new Map();
  #state: LobbyExperienceState = { kind: 'loading' };

  constructor(
    lobby: LobbyGateway,
    messages: MessageGateway,
    roomId: string,
    identity: RoomIdentity | null,
  ) {
    this.#lobby = new LobbyRoomStore(lobby, roomId);
    this.messages = new MessageRoomStore(messages, roomId);
    this.#identity = identity;
  }

  readonly getSnapshot = (): LobbyExperienceState => this.#state;
  readonly retry = (): void => {
    this.#lobby.retry();
    this.messages.retry();
  };
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1)
      this.#detach = [this.#lobby.subscribe(this.#refresh), this.messages.subscribe(this.#refresh)];
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        for (const detach of this.#detach) detach();
        this.#detach = [];
      }
    };
  };
  readonly #refresh = (): void => {
    const lobby = this.#lobby.getSnapshot();
    if (lobby.kind !== 'ready') this.#state = lobby;
    else {
      const messages = this.messages.getSnapshot();
      const humans = roomHumans(
        lobby.room,
        messages.kind === 'ready' ? messages.room.messages : [],
        this.#identity,
      );
      const projection = projectLobbyScene(lobby.room, null, { previous: this.#layout, humans });
      this.#layout = projection.layout;
      this.#state = Object.freeze({ ...lobby, projection });
    }
    for (const listener of this.#listeners) listener();
  };
}
