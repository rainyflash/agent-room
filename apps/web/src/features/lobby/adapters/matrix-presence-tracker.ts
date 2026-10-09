import type { MatrixPresenceObservation, MatrixPresenceState } from '@agent-room/protocol';
import type { MatrixClient, MatrixEvent, SyncState } from 'matrix-js-sdk';
import { z } from 'zod';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import {
  CLIENT_EVENT_EVENT,
  CLIENT_EVENT_SYNC,
  SYNC_STATE_SYNCING,
} from '@/shared/matrix/matrix-sdk-enums';

/** 同时最多问几个人的在线状态。 */
const FETCH_CONCURRENCY = 4;
/** 问失败了，隔多久才再问同一个人。 */
const FETCH_RETRY_MS = 60_000;
/**
 * 上次确认它不离线以后隔了这么久（电脑睡着、断网），就不算一直看着：这段时间里变成离线的，
 * 不知道是哪一刻变的，交给在线状态里的“上次活动”。同步每 30 秒左右回来一次。
 */
const WATCHING_GAP_MS = 120_000;

const presenceSchema = z.object({
  presence: z.enum(['online', 'unavailable', 'offline']),
  last_active_ago: z.number().int().nonnegative().optional(),
});

type ReportedPresence = {
  readonly state: MatrixPresenceState;
  readonly lastActiveAgoMs?: number;
};

type PresenceEntry = MatrixPresenceObservation & {
  /** 最近一次确认它不离线：收到在线或离开，或者之后每次同步成功。离线时没有。 */
  readonly confirmedAtUnixMs?: number;
};

export type AgentPresenceSource = {
  /**
   * 这些人此刻的 Matrix 在线状态，还不知道的不在结果里。不知道的在后台各问一次
   * （`GET /presence/{userId}/status`），问到了通知订阅者。
   */
  observe(userIds: readonly string[]): ReadonlyMap<string, MatrixPresenceObservation>;
  subscribe(listener: () => void): () => void;
};

/**
 * Agent 的 Matrix 在线状态（`specs/agent-liveness/design.md`）。写名片的 Agent 在不在线、
 * 在不在等消息都看这里。
 *
 * 只认这次打开以后从服务器拿到的：同步里的 `m.presence`，和按需问到的。SDK 会把在线状态存进
 * 本机缓存、下次打开时恢复，恢复出来的“上次活动”按恢复的时刻算，不准，所以不用 SDK 的 `User`。
 * 首次同步只带不离线的人，从缓存接着同步只带变了的人，没拿到的 Agent 各问一次。
 */
export class MatrixPresenceTracker implements AgentPresenceSource {
  readonly #clients: MatrixClientSource;
  readonly #now: () => number;
  readonly #listeners = new Set<() => void>();
  readonly #entries = new Map<string, PresenceEntry>();
  readonly #failedAt = new Map<string, number>();
  readonly #inFlight = new Set<string>();
  readonly #queued = new Set<string>();
  #client: MatrixClient | null = null;
  #detach: (() => void) | null = null;

  constructor(clients: MatrixClientSource, options: { readonly now?: () => number } = {}) {
    this.#clients = clients;
    this.#now = options.now ?? Date.now;
    // 换了客户端马上跟上，不等有人来读：首次同步里的在线状态不能漏掉。
    clients.subscribe(() => {
      this.#bind(clients.current());
    });
    this.#bind(clients.current());
  }

  observe(userIds: readonly string[]): ReadonlyMap<string, MatrixPresenceObservation> {
    this.#bind(this.#clients.current());
    const observed = new Map<string, MatrixPresenceObservation>();
    for (const userId of userIds) {
      const entry = this.#entries.get(userId);
      if (entry === undefined) this.#request(userId);
      else observed.set(userId, observation(entry));
    }
    this.#pump();
    return observed;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  /** 换了账户（或退出）就不再听旧客户端，记下的在线状态和没问完的一并丢掉。 */
  #bind(client: MatrixClient | null): void {
    if (client === this.#client) return;
    this.#detach?.();
    this.#detach = null;
    this.#client = client;
    this.#entries.clear();
    this.#failedAt.clear();
    this.#inFlight.clear();
    this.#queued.clear();
    if (client === null) return;
    client.on(CLIENT_EVENT_EVENT, this.#onEvent);
    client.on(CLIENT_EVENT_SYNC, this.#onSync);
    this.#detach = () => {
      client.removeListener(CLIENT_EVENT_EVENT, this.#onEvent);
      client.removeListener(CLIENT_EVENT_SYNC, this.#onSync);
    };
  }

  readonly #onEvent = (event: MatrixEvent): void => {
    if (event.getType() !== 'm.presence') return;
    const userId = event.getSender();
    const reported = parsePresence(event.getContent());
    if (userId === undefined || reported === null) return;
    this.#record(userId, reported, this.#now());
  };

  /**
   * 每次同步成功，就确认了没在这次同步里变的人还是原来的样子。从缓存读出的那次不算：
   * 它只发“准备好了”，不发“同步中”。
   */
  readonly #onSync = (state: SyncState): void => {
    if (state !== SYNC_STATE_SYNCING) return;
    const now = this.#now();
    for (const [userId, entry] of this.#entries) {
      if (entry.state !== 'offline')
        this.#entries.set(userId, { ...entry, confirmedAtUnixMs: now });
    }
  };

  #record(userId: string, reported: ReportedPresence, at: number): void {
    const lastActive =
      reported.lastActiveAgoMs === undefined
        ? {}
        : { lastActiveAtUnixMs: at - reported.lastActiveAgoMs };
    if (reported.state !== 'offline') {
      this.#entries.set(userId, { state: reported.state, ...lastActive, confirmedAtUnixMs: at });
      return;
    }
    // 一直看着它不离线、这次同步说它离线了，离线就从这一刻算；没一直看着，不知道是哪一刻变的。
    const previous = this.#entries.get(userId);
    const offlineSeenAtUnixMs =
      previous?.state === 'offline'
        ? previous.offlineSeenAtUnixMs
        : previous?.confirmedAtUnixMs !== undefined &&
            at - previous.confirmedAtUnixMs <= WATCHING_GAP_MS
          ? at
          : undefined;
    this.#entries.set(userId, {
      state: 'offline',
      ...lastActive,
      ...(offlineSeenAtUnixMs === undefined ? {} : { offlineSeenAtUnixMs }),
    });
  }

  #request(userId: string): void {
    if (this.#inFlight.has(userId) || this.#queued.has(userId)) return;
    const failedAt = this.#failedAt.get(userId);
    if (failedAt !== undefined && this.#now() - failedAt < FETCH_RETRY_MS) return;
    this.#queued.add(userId);
  }

  #pump(): void {
    const client = this.#client;
    if (client === null) return;
    for (const userId of this.#queued) {
      if (this.#inFlight.size >= FETCH_CONCURRENCY) return;
      this.#queued.delete(userId);
      // 排队的时候同步里可能已经带来了。
      if (this.#entries.has(userId)) continue;
      this.#inFlight.add(userId);
      void this.#fetch(client, userId);
    }
  }

  async #fetch(client: MatrixClient, userId: string): Promise<void> {
    let reported: ReportedPresence | null;
    try {
      reported = parsePresence(await client.getPresence(userId));
    } catch {
      reported = null;
    }
    // 问的时候换了账户：新客户端的记录已经从头开始，这个结果不算数。
    if (client !== this.#client) return;
    this.#inFlight.delete(userId);
    if (reported === null) {
      this.#failedAt.set(userId, this.#now());
    } else if (!this.#entries.has(userId)) {
      // 问的时候同步里来了新的，就以同步的为准。
      this.#record(userId, reported, this.#now());
      for (const listener of this.#listeners) listener();
    }
    this.#pump();
  }
}

function parsePresence(content: unknown): ReportedPresence | null {
  const parsed = presenceSchema.safeParse(content);
  if (!parsed.success) return null;
  const ago = parsed.data.last_active_ago;
  return Object.freeze({
    state: parsed.data.presence,
    ...(ago === undefined ? {} : { lastActiveAgoMs: ago }),
  });
}

function observation(entry: PresenceEntry): MatrixPresenceObservation {
  return Object.freeze({
    state: entry.state,
    ...(entry.offlineSeenAtUnixMs === undefined
      ? {}
      : { offlineSeenAtUnixMs: entry.offlineSeenAtUnixMs }),
    ...(entry.lastActiveAtUnixMs === undefined
      ? {}
      : { lastActiveAtUnixMs: entry.lastActiveAtUnixMs }),
  });
}
