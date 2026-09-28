import {
  createClient,
  Direction,
  Filter,
  MatrixEvent,
  MemoryStore,
  Room,
  type IEvent,
  type MatrixClient,
} from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import {
  MatrixSdkMessageSource,
  matrixMessageHistoryFilterId,
  matrixMessagePreviewEventType,
  matrixMessageRevisionEventType,
  matrixModerationNoticeEventType,
  type MatrixFilterClass,
  type MatrixMessageRoomSnapshot,
  type MatrixMessageSourceRead,
} from './matrix-message-source';
import { matrixAgentStatusEventType } from '@/features/lobby/adapters/matrix-lobby-source';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

const ROOM_ID = '!public:agent-room.test';
const legacyPreviewType = ['org', 'agentroom', 'message', 'preview', 'v1'].join('.');

describe('MatrixSdkMessageSource', () => {
  it('expands cached history before requesting another real page', async () => {
    const events = Array.from({ length: 600 }, (_, index) =>
      new FakeEvent(matrixMessagePreviewEventType, `$event${String(index)}`, index).asEvent(),
    );
    const { client, room, source } = harness(new FakeRoom(events, [], 'older'));
    expect((await source.loadOlder(ROOM_ID)).ok).toBe(true);
    expect(client.paginateEventTimeline).not.toHaveBeenCalled();
    expect(windowSize(source.read(ROOM_ID))).toBe(400);
    await source.loadOlder(ROOM_ID);
    await source.loadOlder(ROOM_ID);
    expect(client.paginateEventTimeline).toHaveBeenCalledExactlyOnceWith(room.history().live, {
      backwards: true,
      limit: 200,
    });
    expect(client.scrollback).not.toHaveBeenCalled();
  });

  it('coalesces pagination and rejects results after membership or account changes', async () => {
    const room = new FakeRoom([], [], 'older');
    const { client, registry, source } = harness(room);
    let finish: () => void = () => undefined;
    client.paginateEventTimeline.mockImplementation(
      () =>
        new Promise<boolean>((resolve) => {
          finish = () => {
            resolve(true);
          };
        }),
    );
    const pending = source.loadOlder(ROOM_ID);
    expect(source.loadOlder(ROOM_ID)).toBe(pending);
    await vi.waitFor(() => {
      expect(client.paginateEventTimeline).toHaveBeenCalledOnce();
    });
    room.membership = 'leave';
    finish();
    expect(await pending).toEqual({
      ok: false,
      error: { code: 'history.session_changed', retryable: true },
    });
    expect(source.read(ROOM_ID).kind).toBe('room-not-joined');
    room.membership = 'join';
    const oldAccount = source.loadOlder(ROOM_ID);
    await vi.waitFor(() => {
      expect(client.paginateEventTimeline).toHaveBeenCalledTimes(2);
    });
    registry.replace(new FakeClient(new FakeRoom([])).asClient());
    finish();
    expect((await oldAccount).ok).toBe(false);
    expect(windowSize(source.read(ROOM_ID))).toBe(200);
  });

  it('reports paging failures without growing the search coverage', async () => {
    const { client, source } = harness(new FakeRoom([], [], 'older'));
    client.paginateEventTimeline.mockRejectedValue(new Error('offline'));
    expect(await source.loadOlder(ROOM_ID)).toEqual({
      ok: false,
      error: { code: 'history.load_failed', retryable: true },
    });
    expect(windowSize(source.read(ROOM_ID))).toBe(200);
  });

  it('只复制房间历史里的 Agent Room 消息事件，Agent 在线状态不算', async () => {
    const preview = new FakeEvent(matrixMessagePreviewEventType, '$preview', 20).asEvent();
    const revision = new FakeEvent(matrixMessageRevisionEventType, '$revision', 30).asEvent();
    const ordinary = new FakeEvent('m.room.message', '$ordinary', 10).asEvent();
    const status = new FakeEvent(matrixAgentStatusEventType, '$status', 33).asEvent();
    const future = new FakeEvent(
      'io.github.rainyflash.agentroom.message.future.v9',
      '$future',
      35,
    ).asEvent();
    const legacy = new FakeEvent(legacyPreviewType, '$legacy', 36).asEvent();
    const moderation = new FakeEvent(matrixModerationNoticeEventType, '$moderation', 40).asEvent();
    const room = new FakeRoom([ordinary, preview, revision, status, future, legacy], [moderation]);
    const loading = deferred<MatrixFilterClass>();
    const { source } = harness(room, () => loading.promise);
    const expected = {
      kind: 'ready',
      room: {
        windowSize: 200,
        hasOlder: false,
        roomId: ROOM_ID,
        timelineEvents: [
          timelineEvent(matrixMessagePreviewEventType, '$preview', 20),
          timelineEvent(matrixMessageRevisionEventType, '$revision', 30),
          timelineEvent('io.github.rainyflash.agentroom.message.future.v9', '$future', 35),
          timelineEvent(legacyPreviewType, '$legacy', 36),
          timelineEvent(matrixModerationNoticeEventType, '$moderation', 40),
        ],
      },
    };

    // SDK 模块加载前读不带过滤器的时间线，状态事件同样不进快照。
    expect(source.read(ROOM_ID)).toEqual(expected);
    expect(room.filters).toEqual([]);
    loading.resolve(Filter);
    await vi.waitFor(() => {
      source.read(ROOM_ID);
      expect(room.filters).toHaveLength(1);
    });
    expect(source.read(ROOM_ID)).toEqual(expected);
    expect(room.history().live.events).not.toContain(status);
    expect(room.history().live.events).not.toContain(ordinary);
  });

  it('往回翻页的过滤器排除在线状态，并按线上类型留下解密失败的加密事件', async () => {
    const room = new FakeRoom([]);
    const { source } = harness(room);
    await source.loadOlder(ROOM_ID);
    const filter = room.filters[0];
    if (filter === undefined) throw new Error('没有创建历史过滤器');

    expect(filter.filterId).toBe(matrixMessageHistoryFilterId);
    // /messages 直接带这份定义，不需要先在服务器上创建过滤器。
    expect(filter.getRoomTimelineFilterComponent()?.toJSON()).toEqual({
      types: ['io.github.rainyflash.agentroom.*', 'org.agentroom.*', 'm.room.encrypted'],
      not_types: [matrixAgentStatusEventType],
    });
    const undecryptable = new FakeEvent('m.room.message', '$utd', 1, {
      wireType: 'm.room.encrypted',
    }).asEvent();
    const preview = new FakeEvent(matrixMessagePreviewEventType, '$preview', 2).asEvent();
    const legacy = new FakeEvent(legacyPreviewType, '$legacy', 3).asEvent();
    const status = new FakeEvent(matrixAgentStatusEventType, '$status', 4).asEvent();
    const ordinary = new FakeEvent('m.room.member', '$member', 5).asEvent();
    expect(filter.filterRoomTimeline([undecryptable, preview, legacy, status, ordinary])).toEqual([
      undecryptable,
      preview,
      legacy,
    ]);
  });

  it('SDK 模块加载前先读实时时间线，加载好后只通知读过的房间', async () => {
    const room = new FakeRoom([new FakeEvent(matrixMessagePreviewEventType, '$a', 1).asEvent()]);
    const loading = deferred<MatrixFilterClass>();
    const { source } = harness(room, () => loading.promise);
    const listener = vi.fn();
    const other = vi.fn();
    source.subscribe(ROOM_ID, listener);
    source.subscribe('!other:agent-room.test', other);

    expect(source.read(ROOM_ID).kind).toBe('ready');
    expect(room.filters).toEqual([]);
    loading.resolve(Filter);
    await vi.waitFor(() => {
      expect(listener).toHaveBeenCalledOnce();
    });
    expect(other).not.toHaveBeenCalled();
    source.read(ROOM_ID);
    expect(room.filters).toHaveLength(1);
  });

  it('SDK 模块加载失败时继续按实时时间线读，加载更早消息报可重试的失败', async () => {
    const room = new FakeRoom([new FakeEvent(matrixMessagePreviewEventType, '$a', 1).asEvent()]);
    const load = vi
      .fn<() => Promise<MatrixFilterClass>>()
      .mockRejectedValueOnce(new Error('chunk failed'))
      .mockResolvedValue(Filter);
    const { source } = harness(room, load);

    expect(source.read(ROOM_ID).kind).toBe('ready');
    expect(await source.loadOlder(ROOM_ID)).toEqual({
      ok: false,
      error: { code: 'history.load_failed', retryable: true },
    });
    expect(room.filters).toEqual([]);
    expect((await source.loadOlder(ROOM_ID)).ok).toBe(true);
    expect(room.filters).toHaveLength(1);
  });

  it('从最早一段接着往回翻，解密完这一页再通知', async () => {
    const newest = new FakeEvent(matrixMessagePreviewEventType, '$newest', 30).asEvent();
    const middle = new FakeEvent(matrixMessagePreviewEventType, '$middle', 20).asEvent();
    const room = new FakeRoom([newest]);
    const { client, source } = harness(room);
    await source.loadOlder(ROOM_ID);
    // 同步断档后 SDK 另起一段实时时间线；往回翻页接上以后，旧的那段成为它的邻居。
    const live = room.history().live;
    const earlier = new FakeTimeline([middle], 'before-middle');
    live.previous = earlier;
    earlier.next = live;
    live.token = 'stale-live-token';
    const encrypted = new FakeEvent('m.room.encrypted', '$oldest', 10, {
      wireType: 'm.room.encrypted',
    });
    const decryption = deferred<undefined>();
    encrypted.decryption = decryption.promise;
    client.paginateEventTimeline.mockImplementation((timeline: FakeTimeline) => {
      timeline.events.unshift(encrypted.asEvent());
      timeline.token = null;
      return Promise.resolve(false);
    });
    const listener = vi.fn();
    source.subscribe(ROOM_ID, listener);
    let settled = false;
    const loading = source.loadOlder(ROOM_ID).then((result) => {
      settled = true;
      return result;
    });

    await vi.waitFor(() => {
      expect(client.decryptEventIfNeeded).toHaveBeenCalledWith(encrypted.asEvent());
    });
    expect(client.paginateEventTimeline).toHaveBeenCalledExactlyOnceWith(earlier, {
      backwards: true,
      limit: 200,
    });
    expect(settled).toBe(false);
    expect(listener).not.toHaveBeenCalled();
    encrypted.decryptAs(matrixMessagePreviewEventType);
    decryption.resolve(undefined);
    expect((await loading).ok).toBe(true);
    expect(listener).toHaveBeenCalledOnce();
    const read = source.read(ROOM_ID);
    expect(read.kind === 'ready' && read.room.hasOlder).toBe(false);
    expect(
      read.kind === 'ready' ? read.room.timelineEvents.map((event) => event.eventId) : [],
    ).toEqual(['$oldest', '$middle', '$newest']);
  });

  it('房间没变时交回同一个快照，事件、令牌、窗口和治理状态一变就重建', async () => {
    const preview = new FakeEvent(matrixMessagePreviewEventType, '$preview', 10);
    const encrypted = new FakeEvent('m.room.encrypted', '$encrypted', 20, {
      wireType: 'm.room.encrypted',
    });
    const events = Array.from({ length: 500 }, (_, index) =>
      new FakeEvent(matrixMessagePreviewEventType, `$bulk${String(index)}`, index).asEvent(),
    );
    const room = new FakeRoom([...events, preview.asEvent(), encrypted.asEvent()], [], 'older');
    const { source } = harness(room);
    await source.loadOlder(ROOM_ID);
    const live = room.history().live;
    let current = snapshot(source.read(ROOM_ID));
    const changed = (): boolean => {
      const next = snapshot(source.read(ROOM_ID));
      const different = next !== current;
      current = next;
      return different;
    };

    expect(changed()).toBe(false);
    encrypted.decryptAs(matrixMessagePreviewEventType);
    expect(changed()).toBe(true);
    preview.redacted = true;
    expect(changed()).toBe(true);
    preview.eventId = '$preview-remote';
    expect(changed()).toBe(true);
    live.events.push(new FakeEvent(matrixMessagePreviewEventType, '$new', 30).asEvent());
    expect(changed()).toBe(true);
    live.token = 'older-still';
    expect(changed()).toBe(true);
    room.addModeration(new FakeEvent(matrixModerationNoticeEventType, '$notice', 40).asEvent());
    expect(changed()).toBe(true);
    expect(changed()).toBe(false);
    // 窗口扩大（本地已缓存的消息够用，不用真的翻页）也会重建快照。
    await source.loadOlder(ROOM_ID);
    expect(changed()).toBe(true);
    expect(current.windowSize).toBe(600);
    expect(changed()).toBe(false);
  });

  it('换账户时清空窗口和快照缓存', async () => {
    const room = new FakeRoom([new FakeEvent(matrixMessagePreviewEventType, '$a', 1).asEvent()]);
    const { registry, source } = harness(room);
    await source.loadOlder(ROOM_ID);
    const before = snapshot(source.read(ROOM_ID));
    expect(snapshot(source.read(ROOM_ID))).toBe(before);

    registry.replace(new FakeClient(room).asClient());

    const after = snapshot(source.read(ROOM_ID));
    expect(after).not.toBe(before);
    expect(after).toEqual(before);
  });

  it('没有 Matrix 客户端或房间时返回明确边界', () => {
    const registry = new MatrixClientRegistry();
    const source = new MatrixSdkMessageSource(registry, () => Promise.resolve(Filter));

    expect(source.read(ROOM_ID)).toEqual({ kind: 'matrix-unavailable' });

    registry.replace(new FakeClient(null).asClient());
    expect(source.read(ROOM_ID)).toEqual({ kind: 'room-not-joined' });
  });

  it('把客户端同步活动转发给订阅者并正确释放', () => {
    const { client, registry, source } = harness(new FakeRoom([]));
    const listener = vi.fn();

    const unsubscribe = source.subscribe(ROOM_ID, listener);
    registry.refresh(client.asClient());
    unsubscribe();
    registry.refresh(client.asClient());

    expect(listener).toHaveBeenCalledOnce();
  });
});

describe('MatrixSdkMessageSource 与真实 Matrix SDK', () => {
  it('按过滤器往回翻页；同步断档后翻回旧记录，两段接上，之前加载的消息都回来', async () => {
    const me = '@me:agent-room.test';
    const pages: unknown[] = [
      { chunk: [rawEvent(matrixMessagePreviewEventType, '$b', 50)], start: 't-live', end: 't-old' },
      { chunk: [], start: 't-old' },
      {
        chunk: [
          rawEvent(matrixMessagePreviewEventType, '$gap', 150),
          rawEvent(matrixMessagePreviewEventType, '$a', 100),
        ],
        start: 't-gap',
        end: 't-before-gap',
      },
    ];
    const requests: URL[] = [];
    const fetchFn = vi.fn<typeof fetch>((input) => {
      requests.push(
        new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url),
      );
      return Promise.resolve(
        new Response(JSON.stringify(pages.shift()), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    });
    const client = createClient({
      accessToken: 'test-only-token',
      baseUrl: 'https://matrix.example.test',
      deviceId: 'DEVICE',
      fetchFn,
      store: new MemoryStore(),
      timelineSupport: true,
      userId: me,
    });
    const room = new Room(ROOM_ID, client, me, { timelineSupport: true });
    room.updateMyMembership('join');
    client.store.storeRoom(room);
    await room.addLiveEvents(
      [
        new MatrixEvent(rawEvent(matrixAgentStatusEventType, '$status', 90, 'instance-a')),
        new MatrixEvent(rawEvent(matrixMessagePreviewEventType, '$a', 100)),
      ],
      { addToState: false },
    );
    room.getLiveTimeline().setPaginationToken('t-live', Direction.Backward);
    const registry = new MatrixClientRegistry();
    registry.replace(client);
    const source = new MatrixSdkMessageSource(registry, () => Promise.resolve(Filter));
    const ids = () => snapshot(source.read(ROOM_ID)).timelineEvents.map((event) => event.eventId);

    expect((await source.loadOlder(ROOM_ID)).ok).toBe(true);
    const first = requests[0];
    expect(first?.pathname).toBe(
      `/_matrix/client/v3/rooms/${encodeURIComponent(ROOM_ID)}/messages`,
    );
    expect(first?.searchParams.get('from')).toBe('t-live');
    expect(first?.searchParams.get('dir')).toBe('b');
    expect(first?.searchParams.get('limit')).toBe('200');
    expect(JSON.parse(first?.searchParams.get('filter') ?? 'null')).toEqual({
      types: ['io.github.rainyflash.agentroom.*', 'org.agentroom.*', 'm.room.encrypted'],
      not_types: [matrixAgentStatusEventType],
    });
    expect(ids()).toEqual(['$b', '$a']);
    expect(snapshot(source.read(ROOM_ID)).hasOlder).toBe(true);
    await source.loadOlder(ROOM_ID);
    expect(snapshot(source.read(ROOM_ID)).hasOlder).toBe(false);

    // 同步断档（limited）：SDK 给每组时间线另起一段实时时间线，旧的一段暂时不在视野里。
    room.resetLiveTimeline('t-gap', 'old-sync-token');
    await room.addLiveEvents(
      [new MatrixEvent(rawEvent(matrixMessagePreviewEventType, '$c', 200))],
      {
        addToState: false,
      },
    );
    expect(ids()).toEqual(['$c']);
    expect(snapshot(source.read(ROOM_ID)).hasOlder).toBe(true);

    // 往回翻到见过的消息时 SDK 把两段接成邻居，但不更新新一段的翻页令牌；
    // 从最早一段读起、从最早一段接着翻，之前加载的记录全部回来，也不会反复请求同一页。
    await source.loadOlder(ROOM_ID);
    expect(requests[2]?.searchParams.get('from')).toBe('t-gap');
    expect(ids()).toEqual(['$b', '$a', '$gap', '$c']);
    expect(snapshot(source.read(ROOM_ID)).hasOlder).toBe(false);
    await source.loadOlder(ROOM_ID);
    expect(fetchFn).toHaveBeenCalledTimes(3);
    // 房间自己的实时时间线没有被翻页动过。
    expect(
      room
        .getLiveTimeline()
        .getEvents()
        .map((event) => event.getId()),
    ).toEqual(['$c']);
  });
});

function rawEvent(
  type: string,
  eventId: string,
  serverTimestamp: number,
  stateKey?: string,
): Partial<IEvent> {
  return {
    content: { eventType: type },
    event_id: eventId,
    origin_server_ts: serverTimestamp,
    room_id: ROOM_ID,
    sender: '@agent:agent-room.test',
    type,
    ...(stateKey === undefined ? {} : { state_key: stateKey }),
  };
}

function harness(
  room: FakeRoom,
  loadFilterClass: () => Promise<MatrixFilterClass> = () => Promise.resolve(Filter),
) {
  const client = new FakeClient(room);
  const registry = new MatrixClientRegistry();
  registry.replace(client.asClient());
  return {
    client,
    registry,
    room,
    source: new MatrixSdkMessageSource(registry, loadFilterClass),
  };
}

function snapshot(read: MatrixMessageSourceRead): MatrixMessageRoomSnapshot {
  if (read.kind !== 'ready') throw new Error(`房间不可读：${read.kind}`);
  return read.room;
}

function windowSize(read: MatrixMessageSourceRead): number | undefined {
  return snapshot(read).windowSize;
}

function timelineEvent(type: string, eventId: string, serverTimestamp: number) {
  return {
    content: { eventType: type },
    endToEndEncrypted: false,
    eventId,
    sender: '@agent:agent-room.test',
    serverTimestamp,
    type,
  };
}

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

class FakeEvent {
  decryption: Promise<undefined> | null = null;
  eventId: string;
  redacted = false;
  type: string;
  readonly #content: { readonly eventType: string };
  readonly #serverTimestamp: number;
  readonly #wireType: string;

  constructor(
    type: string,
    eventId: string,
    serverTimestamp: number,
    options: { readonly wireType?: string } = {},
  ) {
    this.type = type;
    this.eventId = eventId;
    this.#serverTimestamp = serverTimestamp;
    this.#wireType = options.wireType ?? type;
    this.#content = { eventType: type };
  }

  decryptAs(type: string): void {
    this.type = type;
    this.decryption = null;
  }

  asEvent(): MatrixEvent {
    return this as unknown as MatrixEvent;
  }

  getContent(): { readonly eventType: string } {
    return this.type === this.#wireType ? this.#content : { eventType: this.type };
  }

  getId(): string {
    return this.eventId;
  }

  getSender(): string {
    return '@agent:agent-room.test';
  }

  getTs(): number {
    return this.#serverTimestamp;
  }

  getType(): string {
    return this.type;
  }

  getWireType(): string {
    return this.#wireType;
  }

  isBeingDecrypted(): boolean {
    return this.decryption !== null;
  }

  isEncrypted(): boolean {
    return this.#wireType === 'm.room.encrypted';
  }

  isRedacted(): boolean {
    return this.redacted;
  }

  replacingEvent(): MatrixEvent | null {
    return null;
  }
}

class FakeTimeline {
  next: FakeTimeline | null = null;
  previous: FakeTimeline | null = null;

  constructor(
    readonly events: MatrixEvent[],
    public token: string | null = null,
    readonly state?: { readonly events: Map<string, Map<string, MatrixEvent>> },
  ) {}

  getEvents(): MatrixEvent[] {
    return this.events;
  }

  getNeighbouringTimeline(direction: string): FakeTimeline | null {
    return direction === 'b' ? this.previous : this.next;
  }

  getPaginationToken(direction: string): string | null {
    return direction === 'b' ? this.token : null;
  }

  getState(): { readonly events: Map<string, Map<string, MatrixEvent>> } | undefined {
    return this.state;
  }
}

class FakeTimelineSet {
  constructor(readonly live: FakeTimeline) {}

  getLiveTimeline(): FakeTimeline {
    return this.live;
  }
}

class FakeRoom {
  readonly filters: Filter[] = [];
  membership = 'join';
  readonly roomId = ROOM_ID;
  readonly #live: FakeTimeline;
  #history: FakeTimelineSet | null = null;

  constructor(
    events: readonly MatrixEvent[],
    moderation: readonly MatrixEvent[] = [],
    token: string | null = null,
  ) {
    const state = new Map<string, Map<string, MatrixEvent>>();
    this.#live = new FakeTimeline([...events], token, { events: state });
    for (const event of moderation) this.addModeration(event);
  }

  addModeration(event: MatrixEvent): void {
    const state = this.#live.state?.events;
    if (state === undefined) throw new Error('缺少房间状态');
    const notices = state.get(matrixModerationNoticeEventType) ?? new Map<string, MatrixEvent>();
    notices.set(event.getId() ?? '', event);
    state.set(matrixModerationNoticeEventType, notices);
  }

  history(): FakeTimelineSet {
    if (this.#history === null) throw new Error('还没有创建历史时间线');
    return this.#history;
  }

  getLiveTimeline(): FakeTimeline {
    return this.#live;
  }

  getMyMembership(): string {
    return this.membership;
  }

  // 与 SDK 一样：用实时时间线里通过过滤器的事件预先填充，翻页令牌沿用实时时间线的。
  getOrCreateFilteredTimelineSet(filter: Filter): FakeTimelineSet {
    this.filters.push(filter);
    this.#history ??= new FakeTimelineSet(
      new FakeTimeline(filter.filterRoomTimeline([...this.#live.events]), this.#live.token),
    );
    return this.#history;
  }
}

class FakeClient {
  readonly decryptEventIfNeeded = vi.fn(
    (event: FakeEvent) => event.decryption ?? Promise.resolve(),
  );
  readonly paginateEventTimeline = vi.fn(
    (timeline: FakeTimeline, options: { readonly backwards: boolean; readonly limit: number }) => {
      void timeline;
      void options;
      return Promise.resolve(true);
    },
  );
  readonly scrollback = vi.fn();
  readonly #room: FakeRoom | null;

  constructor(room: FakeRoom | null) {
    this.#room = room;
  }

  asClient(): MatrixClient {
    return this as unknown as MatrixClient;
  }

  getRoom(): Room | null {
    return this.#room as unknown as Room | null;
  }
}
