import type {
  EventTimeline,
  Filter,
  MatrixClient,
  MatrixEvent,
  Room,
  RoomState,
} from 'matrix-js-sdk';
import { matrixAgentStatusEventType } from '@/features/lobby/adapters/matrix-lobby-source';
import { DIRECTION_BACKWARD, DIRECTION_FORWARD } from '@/shared/matrix/matrix-sdk-enums';
import { err, ok, type Result } from '@/shared/result';

import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

export const matrixMessagePreviewEventType = 'io.github.rainyflash.agentroom.message.preview.v1';
export const matrixMessageRevisionEventType = 'io.github.rainyflash.agentroom.message.revision.v1';
export const matrixMessagePreviewEventTypeV2 = 'io.github.rainyflash.agentroom.message.preview.v2';
export const matrixMessageRevisionEventTypeV2 =
  'io.github.rainyflash.agentroom.message.revision.v2';
export const matrixModerationNoticeEventType =
  'io.github.rainyflash.agentroom.moderation.notice.v1';
export const matrixAgentRoomEventNamespace = 'io.github.rainyflash.agentroom';
const legacyMatrixAgentRoomEventNamespace = ['org', 'agentroom'].join('.');
const matrixEncryptedEventType = 'm.room.encrypted';

const projectedTimelineEventTypes = new Set([
  matrixMessagePreviewEventType,
  matrixMessagePreviewEventTypeV2,
  matrixMessageRevisionEventType,
  matrixMessageRevisionEventTypeV2,
  matrixModerationNoticeEventType,
]);
const historyPageSize = 200;
const historyLimit = 5000;
/**
 * SDK 只把过滤器 ID 当作房间里这组时间线的键：/messages 请求直接带上过滤器定义，
 * 不需要先在服务器上创建过滤器。
 */
export const matrixMessageHistoryFilterId = 'agent-room.message-history.v1';
/** 同步断档后 SDK 另起时间线，往回翻页接上旧的以后二者互为邻居；这里只防御成环。 */
const maxNeighbouringTimelines = 1_000;

export type MatrixMessageTimelineEvent = {
  readonly content: unknown;
  /** 这台设备解不开时 SDK 给的原因码（DecryptionFailureCode）；解开了或没加密时不给。 */
  readonly decryptionFailure?: string;
  readonly endToEndEncrypted: boolean;
  readonly eventId: string | undefined;
  readonly sender: string | undefined;
  readonly serverTimestamp: number;
  readonly type: string;
};

export type MatrixMessageRoomSnapshot = {
  readonly windowSize?: number;
  readonly hasOlder?: boolean;
  readonly roomId: string;
  readonly timelineEvents: readonly MatrixMessageTimelineEvent[];
};

export type MatrixMessageSourceRead =
  | { readonly kind: 'matrix-unavailable' }
  | { readonly kind: 'room-not-joined' }
  | { readonly kind: 'ready'; readonly room: MatrixMessageRoomSnapshot };

export type MatrixMessageSource = {
  loadOlder?(
    roomId: string,
  ): Promise<Result<void, { readonly code: string; readonly retryable: boolean }>>;
  read(roomId: string): MatrixMessageSourceRead;
  subscribe(roomId: string, listener: () => void): () => void;
};

export type MatrixFilterClass = typeof Filter;

type HistoryResult = Result<void, { readonly code: string; readonly retryable: boolean }>;

/** 快照里每个历史事件的样子；这些都没变，投影出来的结果就不会变。 */
type HistoryEventSignature = {
  readonly encrypted: boolean;
  readonly event: MatrixEvent;
  readonly eventId: string | undefined;
  readonly failure: string | null;
  readonly redacted: boolean;
  readonly replacement: MatrixEvent | null;
  readonly serverTimestamp: number;
  readonly type: string;
};

type CachedRoomSnapshot = {
  readonly history: readonly HistoryEventSignature[];
  readonly moderation: readonly MatrixEvent[];
  readonly snapshot: MatrixMessageRoomSnapshot;
  readonly token: string | null;
  readonly windowSize: number;
};

/**
 * 读 Matrix 房间里的 Agent Room 消息事件。
 *
 * 聊天记录从一组带过滤器的时间线里读、也从那里往回翻页：Agent 的在线状态是频繁改写的状态事件，
 * 每次改写也会进时间线，不过滤的话一页 200 个事件里可能只有几条消息。
 */
export class MatrixSdkMessageSource implements MatrixMessageSource {
  readonly #clients: MatrixClientSource;
  readonly #loadFilterClass: () => Promise<MatrixFilterClass>;
  #client: MatrixClient | null = null;
  #filter: Filter | null = null;
  #filterLoading: Promise<Filter> | null = null;
  readonly #fallbackRooms = new Set<string>();
  readonly #windows = new Map<string, number>();
  readonly #loading = new Map<string, Promise<HistoryResult>>();
  readonly #listeners = new Map<string, Set<() => void>>();
  readonly #snapshots = new Map<string, CachedRoomSnapshot>();

  constructor(
    clients: MatrixClientSource,
    loadFilterClass: () => Promise<MatrixFilterClass> = loadMatrixFilterClass,
  ) {
    this.#clients = clients;
    this.#loadFilterClass = loadFilterClass;
  }

  read(roomId: string): MatrixMessageSourceRead {
    const client = this.#current();
    if (client === null) {
      return { kind: 'matrix-unavailable' };
    }
    const room = client.getRoom(roomId);
    if (room?.getMyMembership() !== 'join') {
      return { kind: 'room-not-joined' };
    }
    const state = room.getLiveTimeline().getState(DIRECTION_FORWARD);
    if (state === undefined) {
      return { kind: 'room-not-joined' };
    }
    return { kind: 'ready', room: this.#snapshot(roomId, this.#historyTimeline(room), state) };
  }

  subscribe(roomId: string, listener: () => void): () => void {
    const listeners = this.#listeners.get(roomId) ?? new Set<() => void>();
    listeners.add(listener);
    this.#listeners.set(roomId, listeners);
    const detach = this.#clients.subscribe(listener);
    return () => {
      detach();
      listeners.delete(listener);
      if (listeners.size === 0) this.#listeners.delete(roomId);
    };
  }

  loadOlder(roomId: string): Promise<HistoryResult> {
    this.#current();
    const pending = this.#loading.get(roomId);
    if (pending) return pending;
    const operation = this.#load(roomId).catch(() =>
      err({ code: 'history.load_failed', retryable: true }),
    );
    this.#loading.set(roomId, operation);
    void operation.finally(() => {
      if (this.#loading.get(roomId) === operation) this.#loading.delete(roomId);
    });
    return operation;
  }

  async #load(roomId: string): Promise<HistoryResult> {
    const client = this.#current();
    const room = client?.getRoom(roomId);
    if (!client || room?.getMyMembership() !== 'join')
      return err({ code: 'history.room_unavailable', retryable: true });
    const currentWindow = this.#windows.get(roomId) ?? historyPageSize;
    if (currentWindow >= historyLimit) return err({ code: 'history.limit', retryable: false });
    try {
      const filter = await this.#historyFilter();
      if (this.#current() !== client || room.getMyMembership() !== 'join')
        return err({ code: 'history.session_changed', retryable: true });
      const history = room.getOrCreateFilteredTimelineSet(filter);
      const earliest = earliestTimeline(history.getLiveTimeline());
      if (
        countPreviews(history.getLiveTimeline()) <= currentWindow &&
        earliest.getPaginationToken(DIRECTION_BACKWARD) !== null
      ) {
        await client.paginateEventTimeline(earliest, {
          backwards: true,
          limit: historyPageSize,
        });
        // 翻页拿到的加密事件要解密完才看得出是不是消息；先通知的话，这批消息要等下一次同步才显示。
        await settleDecryption(client, history.getLiveTimeline());
      }
      if (this.#current() !== client || room.getMyMembership() !== 'join')
        return err({ code: 'history.session_changed', retryable: true });
      const available = countPreviews(history.getLiveTimeline());
      this.#windows.set(
        roomId,
        available > currentWindow
          ? Math.min(currentWindow + historyPageSize, historyLimit)
          : currentWindow,
      );
      this.#notify(roomId);
      return ok(undefined);
    } catch {
      return err({ code: 'history.load_failed', retryable: true });
    }
  }

  /** 过滤器类要等 SDK 模块加载；在那之前先读不带过滤器的实时时间线，加载好了再通知这些房间。 */
  #historyTimeline(room: Room): EventTimeline {
    if (this.#filter !== null) {
      return room.getOrCreateFilteredTimelineSet(this.#filter).getLiveTimeline();
    }
    this.#fallbackRooms.add(room.roomId);
    // 加载失败时仍按不带过滤器的时间线读，下次读取会重试；加载更早消息会把失败报给用户。
    void this.#historyFilter().catch(keepUnfilteredHistory);
    return room.getLiveTimeline();
  }

  #historyFilter(): Promise<Filter> {
    if (this.#filter !== null) return Promise.resolve(this.#filter);
    this.#filterLoading ??= this.#loadFilterClass().then(
      (filterClass) => {
        const filter = createHistoryFilter(filterClass);
        this.#filter = filter;
        const rooms = [...this.#fallbackRooms];
        this.#fallbackRooms.clear();
        for (const roomId of rooms) this.#notify(roomId);
        return filter;
      },
      (error: unknown) => {
        this.#filterLoading = null;
        throw error;
      },
    );
    return this.#filterLoading;
  }

  #snapshot(roomId: string, live: EventTimeline, state: RoomState): MatrixMessageRoomSnapshot {
    const windowSize = this.#windows.get(roomId) ?? historyPageSize;
    const token = earliestTimeline(live).getPaginationToken(DIRECTION_BACKWARD);
    const moderation = state.events.get(matrixModerationNoticeEventType);
    const cached = this.#snapshots.get(roomId);
    if (
      cached?.windowSize === windowSize &&
      cached.token === token &&
      sameModeration(cached.moderation, moderation) &&
      sameHistory(cached.history, live)
    ) {
      return cached.snapshot;
    }
    const next = buildSnapshot(roomId, live, moderation, windowSize, token);
    this.#snapshots.set(roomId, next);
    return next.snapshot;
  }

  #notify(roomId: string): void {
    for (const listener of this.#listeners.get(roomId) ?? []) listener();
  }

  #current(): MatrixClient | null {
    const client = this.#clients.current();
    if (client !== this.#client) {
      this.#client = client;
      this.#windows.clear();
      this.#loading.clear();
      this.#snapshots.clear();
    }
    return client;
  }
}

function keepUnfilteredHistory(error: unknown): void {
  void error;
}

async function loadMatrixFilterClass(): Promise<MatrixFilterClass> {
  // 与会话网关同一个动态入口：有客户端时 SDK 已经加载，这里不会把 SDK 拉进首屏主包。
  const sdk = await import('matrix-js-sdk');
  return sdk.Filter;
}

function createHistoryFilter(filterClass: MatrixFilterClass): Filter {
  /**
   * 服务器按线上类型套用 /messages 过滤器；SDK 在本地却按解密后的类型再筛一遍。
   * 两边不一致时，解密失败（显示成 m.room.message）的事件会被丢掉，密钥晚到、重新解密成功后
   * 也回不来；整页都被丢掉时 SDK 还不前移翻页令牌。所以本地也按线上类型筛。
   */
  class MessageHistoryFilter extends filterClass {
    override filterRoomTimeline(events: MatrixEvent[]): MatrixEvent[] {
      return events.filter(isHistoryTimelineEvent);
    }
  }
  const filter = new MessageHistoryFilter(null, matrixMessageHistoryFilterId);
  filter.setDefinition({
    room: {
      timeline: {
        types: [
          `${matrixAgentRoomEventNamespace}.*`,
          `${legacyMatrixAgentRoomEventNamespace}.*`,
          matrixEncryptedEventType,
        ],
        not_types: [matrixAgentStatusEventType],
      },
    },
  });
  return filter;
}

function buildSnapshot(
  roomId: string,
  live: EventTimeline,
  moderation: ReadonlyMap<string, MatrixEvent> | undefined,
  windowSize: number,
  token: string | null,
): CachedRoomSnapshot {
  const history: HistoryEventSignature[] = [];
  const timelineEvents: MatrixMessageTimelineEvent[] = [];
  forEachHistoryEvent(live, (event) => {
    history.push(signature(event));
    if (isProjectedTimelineEvent(event)) {
      timelineEvents.push(toTimelineEvent(event));
      return;
    }
    // 解不开的事件不知道原本是什么，交给网关汇总成一条提示，不能不声不响地丢掉：
    // 否则房间里全是解不开的消息时，界面只说“还没有消息”。
    const failure = decryptionFailure(event);
    if (failure !== null) timelineEvents.push(toUndecryptableEvent(event, failure));
  });
  const moderationEvents = [...(moderation?.values() ?? [])];
  for (const event of moderationEvents) timelineEvents.push(toTimelineEvent(event));
  return {
    history,
    moderation: moderationEvents,
    snapshot: Object.freeze({
      roomId,
      windowSize,
      hasOlder: token !== null,
      timelineEvents: Object.freeze(timelineEvents),
    }),
    token,
    windowSize,
  };
}

function signature(event: MatrixEvent): HistoryEventSignature {
  return {
    encrypted: event.isEncrypted(),
    event,
    eventId: event.getId(),
    failure: decryptionFailure(event),
    redacted: event.isRedacted(),
    replacement: event.replacingEvent(),
    serverTimestamp: event.getTs(),
    type: event.getType(),
  };
}

/**
 * 逐个比对，不分配数组：解密完成、解密失败后重试成功、失败原因变了（比如后来收到拒绝分发的通知）、
 * 撤回、本地回显换成服务器事件、编辑替换，都会让其中一项变化。
 */
function sameHistory(history: readonly HistoryEventSignature[], live: EventTimeline): boolean {
  let index = 0;
  const same = everyHistoryEvent(live, (event) => {
    const expected = history[index];
    index += 1;
    return (
      expected?.event === event &&
      expected.eventId === event.getId() &&
      expected.type === event.getType() &&
      expected.encrypted === event.isEncrypted() &&
      expected.failure === decryptionFailure(event) &&
      expected.redacted === event.isRedacted() &&
      expected.serverTimestamp === event.getTs() &&
      expected.replacement === event.replacingEvent()
    );
  });
  return same && index === history.length;
}

function sameModeration(
  cached: readonly MatrixEvent[],
  current: ReadonlyMap<string, MatrixEvent> | undefined,
): boolean {
  if (cached.length !== (current?.size ?? 0)) return false;
  let index = 0;
  for (const event of current?.values() ?? []) {
    if (cached[index] !== event) return false;
    index += 1;
  }
  return true;
}

function countPreviews(live: EventTimeline): number {
  let count = 0;
  forEachHistoryEvent(live, (event) => {
    const eventType = event.getType();
    if (
      eventType === matrixMessagePreviewEventType ||
      eventType === matrixMessagePreviewEventTypeV2
    )
      count += 1;
  });
  return count;
}

async function settleDecryption(client: MatrixClient, live: EventTimeline): Promise<void> {
  const pending: Promise<void>[] = [];
  forEachHistoryEvent(live, (event) => {
    if (
      event.isEncrypted() &&
      (event.getType() === matrixEncryptedEventType || event.isBeingDecrypted())
    )
      pending.push(client.decryptEventIfNeeded(event));
  });
  // 解密失败记在事件自己身上（投影时汇总成一条提示），这里只等每个尝试结束。
  await Promise.allSettled(pending);
}

function forEachHistoryEvent(live: EventTimeline, visit: (event: MatrixEvent) => void): void {
  everyHistoryEvent(live, (event) => {
    visit(event);
    return true;
  });
}

/** 从最早的一段起按时间先后访问历史事件，test 返回 false 时停下并返回 false。 */
function everyHistoryEvent(live: EventTimeline, test: (event: MatrixEvent) => boolean): boolean {
  let timeline: EventTimeline | null = earliestTimeline(live);
  for (let hops = 0; timeline !== null && hops <= maxNeighbouringTimelines; hops += 1) {
    for (const event of timeline.getEvents()) {
      if (isHistoryTimelineEvent(event) && !test(event)) return false;
    }
    if (timeline === live) return true;
    timeline = timeline.getNeighbouringTimeline(DIRECTION_FORWARD);
  }
  return true;
}

function earliestTimeline(live: EventTimeline): EventTimeline {
  let timeline = live;
  for (let hops = 0; hops < maxNeighbouringTimelines; hops += 1) {
    const previous = timeline.getNeighbouringTimeline(DIRECTION_BACKWARD);
    if (previous === null) return timeline;
    timeline = previous;
  }
  return timeline;
}

/**
 * 历史时间线收哪些事件，按线上类型判断，和服务器处理过滤器的方式一致。
 * 加密事件解密前看不出是什么，一律先收下，解密后再由 isProjectedTimelineEvent 挑。
 */
function isHistoryTimelineEvent(event: MatrixEvent): boolean {
  const wireType = event.getWireType();
  return wireType === matrixEncryptedEventType || isProjectedEventType(wireType);
}

function isProjectedTimelineEvent(event: MatrixEvent): boolean {
  return isProjectedEventType(event.getType());
}

/** Agent 在线状态由大厅从房间状态读取，不是聊天记录，也不该当成看不懂的联邦事件列出来。 */
function isProjectedEventType(eventType: string): boolean {
  return (
    eventType !== matrixAgentStatusEventType &&
    (projectedTimelineEventTypes.has(eventType) ||
      isNamespacedEvent(eventType, matrixAgentRoomEventNamespace) ||
      isNamespacedEvent(eventType, legacyMatrixAgentRoomEventNamespace))
  );
}

function isNamespacedEvent(eventType: string, namespace: string): boolean {
  return eventType.startsWith(`${namespace}.`);
}

function toTimelineEvent(event: MatrixEvent): MatrixMessageTimelineEvent {
  return Object.freeze({
    content: event.getContent(),
    endToEndEncrypted: event.isEncrypted(),
    eventId: event.getId(),
    sender: event.getSender(),
    serverTimestamp: event.getTs(),
    type: event.getType(),
  });
}

/** 解密失败的事件只留下谁、什么时候、为什么；SDK 替它编的正文不往外交。 */
function toUndecryptableEvent(event: MatrixEvent, failure: string): MatrixMessageTimelineEvent {
  return Object.freeze({
    content: null,
    decryptionFailure: failure,
    endToEndEncrypted: true,
    eventId: event.getId(),
    sender: event.getSender(),
    serverTimestamp: event.getTs(),
    type: event.getWireType(),
  });
}

function decryptionFailure(event: MatrixEvent): string | null {
  if (!event.isDecryptionFailure()) return null;
  return event.decryptionFailureReason ?? 'UNKNOWN_ERROR';
}
