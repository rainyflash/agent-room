import type { MessageRoomStore } from '@/features/messages/application/message-room-store';
import type { RoomMessageSignal } from '@/features/messages/domain/message';

export const roomSpeechLifetimeMs = 14_000;
export type RoomActivity = {
  readonly recent: readonly RoomMessageSignal[];
  readonly unread: number;
};
type ActivityClock = {
  readonly now: () => number;
  readonly schedule: (callback: () => void, delay: number) => () => void;
};
const browserClock: ActivityClock = {
  now: () => Date.now(),
  schedule: (callback, delay) => {
    const timer = setTimeout(callback, delay);
    return () => {
      clearTimeout(timer);
    };
  },
};
const empty: RoomActivity = Object.freeze({ recent: [], unread: 0 });

/** 未读和短暂气泡都是同一公开房间时间线的派生状态。 */
export class RoomActivityStore {
  readonly #messages: MessageRoomStore;
  readonly #selfId: string | null;
  readonly #clock: ActivityClock;
  readonly #startedAt: number;
  readonly #listeners = new Set<() => void>();
  #known: Set<string> | null = null;
  #unread = new Set<string>();
  #visible = false;
  #detach: (() => void) | null = null;
  #cancelExpiry: (() => void) | null = null;
  #state: RoomActivity = empty;

  constructor(messages: MessageRoomStore, selfId: string | null, clock = browserClock) {
    this.#messages = messages;
    this.#selfId = selfId;
    this.#clock = clock;
    this.#startedAt = clock.now();
  }
  readonly getSnapshot = (): RoomActivity => this.#state;
  readonly setVisible = (visible: boolean): void => {
    this.#visible = visible;
    if (visible) this.#unread.clear();
    this.#refresh();
  };
  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    if (this.#listeners.size === 1) {
      this.#detach = this.#messages.subscribe(this.#refresh);
      this.#refresh();
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detach?.();
        this.#detach = null;
        this.#cancelExpiry?.();
        this.#cancelExpiry = null;
      }
    };
  };
  readonly #refresh = (): void => {
    this.#cancelExpiry?.();
    this.#cancelExpiry = null;
    const source = this.#messages.getSnapshot();
    if (source.kind !== 'ready') {
      this.#state = empty;
      this.#unread.clear();
    } else {
      const now = this.#clock.now();
      const messages = source.room.messages.filter(
        (message) =>
          message.roomId === this.#messages.roomId &&
          message.lifecycle === 'active' &&
          message.preview?.conversation !== undefined,
      );
      const activeIds = new Set(messages.map((message) => message.messageId));
      this.#unread = new Set([...this.#unread].filter((id) => activeIds.has(id)));
      for (const message of messages) {
        if (
          this.#known !== null &&
          !this.#visible &&
          !this.#known.has(message.messageId) &&
          message.actor.matrixUserId !== this.#selfId &&
          message.serverTimestamp >= this.#startedAt - 5000
        )
          this.#unread.add(message.messageId);
      }
      this.#known = new Set(
        [...(this.#known ?? []), ...messages.map((message) => message.messageId)].slice(-1000),
      );
      const recent = messages
        .filter(
          (message) =>
            message.serverTimestamp <= now + 2000 &&
            message.serverTimestamp + roomSpeechLifetimeMs > now,
        )
        .toSorted((a, b) => b.serverTimestamp - a.serverTimestamp);
      this.#state = Object.freeze({ recent: Object.freeze(recent), unread: this.#unread.size });
      const nextExpiry = recent.reduce(
        (earliest, message) => Math.min(earliest, message.serverTimestamp + roomSpeechLifetimeMs),
        Number.POSITIVE_INFINITY,
      );
      if (Number.isFinite(nextExpiry) && this.#listeners.size > 0)
        this.#cancelExpiry = this.#clock.schedule(this.#refresh, Math.max(1, nextExpiry - now));
    }
    for (const listener of this.#listeners) listener();
  };
}
