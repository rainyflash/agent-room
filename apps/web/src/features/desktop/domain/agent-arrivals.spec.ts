import { describe, expect, it } from 'vitest';
import { hostBaseline, hostSessionArrivals, mergeArrivals, rosterArrivals } from './agent-arrivals';
import type { HostSessionDiagnostics } from './desktop-runtime';

const catalogId = '0198b601-77a2-7f41-b4f4-940f291951b8';
const room = { roomId: '!here:matrix.test', catalogId };

function session(
  sessionId: string,
  overrides: Partial<HostSessionDiagnostics> & {
    readonly state?: HostSessionDiagnostics['session']['state'];
  } = {},
): HostSessionDiagnostics {
  const { state = 'ready', ...rest } = overrides;
  return {
    displayName: 'Scout',
    roomId: room.roomId,
    session: {
      sessionId,
      state,
      agentId: `0198b601-77a6-7bb8-83eb-${sessionId.slice(-12)}`,
      errorCode: null,
    },
    lastInboxReadAgoMs: null,
    lastMessageReceivedAgoMs: null,
    lastMessageSentAgoMs: null,
    ...rest,
  };
}

const id = (suffix: number) => `0198b601-77a6-7bb8-83eb-${String(suffix).padStart(12, '0')}`;

describe('agent arrivals', () => {
  it('打开对话框时已有的会话不算刚进来的', () => {
    const sessions = [session(id(1)), session(id(2))];
    const arrivals = hostSessionArrivals(sessions, hostBaseline([session(id(1))]), room);
    expect(arrivals.map((arrival) => arrival.key)).toEqual([id(2)]);
  });

  it('已经接入的 Agent 后来进了这个房间也算到了，打开时它就在这里的不算', () => {
    const elsewhere = '!weekly:matrix.test';
    const before = [
      session(id(1), { roomId: elsewhere, rooms: [] }),
      session(id(2), { roomId: elsewhere, rooms: [] }),
      session(id(3), { roomId: elsewhere, rooms: [{ catalogId, roomId: room.roomId }] }),
      session(id(4), { roomId: elsewhere }),
    ];
    const baseline = hostBaseline(before);
    const now = [
      session(id(1), { roomId: elsewhere, rooms: [{ catalogId, roomId: room.roomId }] }),
      session(id(2), {
        roomId: elsewhere,
        rooms: [{ catalogId, roomId: '!other-instance:matrix.test' }],
      }),
      session(id(3), { roomId: elsewhere, rooms: [{ catalogId, roomId: room.roomId }] }),
      session(id(4), { roomId: elsewhere }),
    ];
    const arrivals = hostSessionArrivals(now, baseline, room);
    expect(arrivals.map((arrival) => [arrival.key, arrival.existing, arrival.elsewhere])).toEqual([
      [`${id(1)}:${room.roomId}`, true, false],
      [`${id(2)}:${room.roomId}`, true, true],
    ]);
    // 从“我的 Agent”打开（没有房间）：已有的 Agent 换房间不算新来的。
    expect(hostSessionArrivals(now, baseline, null)).toEqual([]);
  });

  it('按房间归类：进了这里、正往这里来、进了同一个大厅的另一间，别的房间不列', () => {
    const sessions = [
      session(id(1)),
      session(id(2), { roomId: null, state: 'starting', requestedRoom: { catalogId } }),
      session(id(3), {
        roomId: null,
        state: 'starting',
        requestedRoom: { catalogId, roomId: room.roomId },
      }),
      session(id(4), { roomId: '!other-instance:matrix.test', requestedRoom: { catalogId } }),
      session(id(5), { roomId: '!unrelated:matrix.test' }),
      session(id(6), {
        roomId: null,
        state: 'starting',
        requestedRoom: { catalogId, roomId: '!other-instance:matrix.test' },
      }),
    ];
    const arrivals = hostSessionArrivals(sessions, new Map(), room);
    expect(arrivals.map((arrival) => [arrival.key, arrival.state, arrival.elsewhere])).toEqual([
      [id(1), 'ready', false],
      [id(2), 'starting', false],
      [id(3), 'starting', false],
      [id(4), 'ready', true],
    ]);
  });

  it('没有房间时（从“我的 Agent”打开）新会话都算', () => {
    const sessions = [session(id(1), { roomId: '!anywhere:matrix.test' }), session(id(2))];
    expect(hostSessionArrivals(sessions, new Map(), null)).toHaveLength(2);
  });

  it('35 秒内取过信才算正在看消息；没能进来的带上错误码', () => {
    const [reading, idle, failed] = hostSessionArrivals(
      [
        session(id(1), { lastInboxReadAgoMs: 34_999 }),
        session(id(2), { lastInboxReadAgoMs: 35_000 }),
        {
          ...session(id(3)),
          session: {
            sessionId: id(3),
            state: 'failed',
            agentId: null,
            errorCode: 'bridge.join_rejected',
          },
        },
      ],
      new Map(),
      room,
    );
    expect(reading?.active).toBe(true);
    expect(idle?.active).toBe(false);
    expect(failed).toMatchObject({ state: 'failed', errorCode: 'bridge.join_rejected' });
  });

  it('房间名单里新出现的 Agent 算进来了；本机会话也看到它时只留本机那条', () => {
    const present = [
      { agentId: 'resident', displayName: 'Resident' },
      { agentId: 'visitor', displayName: 'Web visitor' },
      { agentId: session(id(1)).session.agentId ?? '', displayName: 'Scout' },
    ];
    const roster = rosterArrivals(present, new Set(['resident']));
    expect(roster.map((arrival) => arrival.key)).toEqual([
      'room:visitor',
      `room:${present[2]?.agentId ?? ''}`,
    ]);
    const host = hostSessionArrivals([session(id(1))], new Map(), room);
    expect(mergeArrivals(host, roster).map((arrival) => arrival.key)).toEqual([
      id(1),
      'room:visitor',
    ]);
  });
});
