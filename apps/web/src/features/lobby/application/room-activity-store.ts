import type { MessageRoomStore } from '@/features/messages/application/message-room-store';
import type { RoomMessageSignal } from '@/features/messages/domain/message';

export const roomSpeechLifetimeMs = 14_000;
export type RoomActivity = {
  readonly recent: readonly RoomMessageSignal[];
  /** 主房间的对话不在眼前时来的话：标在回去的地方。 */
  readonly unread: number;
  /** 对话在眼前、窗口却不在前台时来的话：只标在窗口的标题上，回到前台就清掉。 */
  readonly away: number;
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
const empty: RoomActivity = Object.freeze({ recent: [], unread: 0, away: 0 });

/** 未读和短暂气泡都是同一公开房间时间线的派生状态。 */
export class RoomActivityStore {
  readonly #messages: MessageRoomStore;
  readonly #selfId: string | null;
  readonly #clock: ActivityClock;
  readonly #startedAt: number;
  readonly #listeners = new Set<() => void>();
  #known: Set<string> | null = null;
  #unread = new Set<string>();
  #away = new Set<string>();
  #visible = false;
  #foreground = true;
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
  /** 窗口到了前台或者离开了前台。回到前台时，不在前台那阵子来的话算看到了。 */
  readonly setForeground = (foreground: boolean): void => {
    if (this.#foreground === foreground) return;
    this.#foreground = foreground;
    if (foreground) this.#away.clear();
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
      this.#away.clear();
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
      this.#away = new Set([...this.#away].filter((id) => activeIds.has(id)));
      for (const message of messages) {
        if (
          this.#known === null ||
          this.#known.has(message.messageId) ||
          message.actor.matrixUserId === this.#selfId ||
          message.serverTimestamp < this.#startedAt - 5000
        )
          continue;
        if (!this.#visible) this.#unread.add(message.messageId);
        else if (!this.#foreground) this.#away.add(message.messageId);
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
      this.#state = Object.freeze({
        recent: Object.freeze(recent),
        unread: this.#unread.size,
        away: this.#away.size,
      });
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
