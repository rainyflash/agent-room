import type { HostSessionDiagnostics } from './desktop-runtime';

/**
 * 打开“接入 Agent”之后新来的一个 Agent。本机的 Agent 从本机会话看得到进度（正在进来、进来了、
 * 没能进来、已离开）；网络 Agent 只在它出现在房间里时看得到，所以只有“进来了”。
 */
export type AgentArrival = {
  readonly key: string;
  readonly agentId: string | null;
  readonly displayName: string;
  readonly state: 'starting' | 'ready' | 'failed' | 'closed';
  /** 进了同一个大厅的另一间（公共大厅人多时会分成几间）。 */
  readonly elsewhere: boolean;
  readonly errorCode: string | null;
  /** 最近 35 秒内取过信：它正在看消息。 */
  readonly active: boolean;
  /** 打开对话框时就已经接入的 Agent，后来进了这个房间。 */
  readonly existing: boolean;
};

export type ArrivalRoom = { readonly roomId: string; readonly catalogId?: string } | null;

/** 房间里已经在场的 Agent。 */
export type PresentAgent = { readonly agentId: string; readonly displayName: string };

/** 与桌面诊断一致：最近 35 秒内有取信才算正在看消息。 */
const ACTIVE_WINDOW_MS = 35_000;

/** 打开对话框时本机已有的会话，和它们那时在的房间（Matrix 房间 ID）。 */
export type HostBaseline = ReadonlyMap<string, ReadonlySet<string>>;

export function hostBaseline(sessions: readonly HostSessionDiagnostics[]): HostBaseline {
  return new Map(sessions.map((entry) => [entry.session.sessionId, sessionRooms(entry)]));
}

/**
 * 对话框打开后到了的本机 Agent。新会话：有房间时只算进了这个房间、或正往这个房间去的；进了
 * 同一个大厅另一间的也列出来，标明在别处；没有房间时（从“我的 Agent”打开）都算。已有的会话：
 * 同一个 Agent 进了这个房间也算到了（`specs/agent-identity/design.md`“网页”），看诊断里的
 * `rooms`；打开时它就在这里的不算。
 */
export function hostSessionArrivals(
  sessions: readonly HostSessionDiagnostics[],
  baseline: HostBaseline,
  room: ArrivalRoom,
): AgentArrival[] {
  const arrivals: AgentArrival[] = [];
  for (const entry of sessions) {
    const before = baseline.get(entry.session.sessionId);
    const placement =
      before === undefined ? placeInRoom(entry, room) : enteredRoom(entry, before, room);
    if (placement === 'unrelated') continue;
    arrivals.push({
      key:
        before === undefined
          ? entry.session.sessionId
          : `${entry.session.sessionId}:${room?.roomId ?? ''}`,
      agentId: entry.session.agentId,
      displayName: entry.displayName,
      state: entry.session.state,
      elsewhere: placement === 'elsewhere',
      errorCode: entry.session.errorCode,
      active: entry.lastInboxReadAgoMs !== null && entry.lastInboxReadAgoMs < ACTIVE_WINDOW_MS,
      existing: before !== undefined,
    });
  }
  return arrivals;
}

/** 已有的会话打开对话框以后又进了这个房间（或者同一个大厅的另一间）。 */
function enteredRoom(
  entry: HostSessionDiagnostics,
  before: ReadonlySet<string>,
  room: ArrivalRoom,
): 'here' | 'elsewhere' | 'unrelated' {
  if (room === null || before.has(room.roomId)) return 'unrelated';
  if (sessionRooms(entry).has(room.roomId)) return 'here';
  const sameLobby = (entry.rooms ?? []).some(
    (entered) => entered.catalogId === room.catalogId && !before.has(entered.roomId),
  );
  return sameLobby ? 'elsewhere' : 'unrelated';
}

/** 会话的 Agent 在的房间：开会话的那个，加上后来进的。 */
function sessionRooms(entry: HostSessionDiagnostics): Set<string> {
  const rooms = new Set((entry.rooms ?? []).map((entered) => entered.roomId));
  if (entry.roomId != null) rooms.add(entry.roomId);
  return rooms;
}

function placeInRoom(
  entry: HostSessionDiagnostics,
  room: ArrivalRoom,
): 'here' | 'elsewhere' | 'unrelated' {
  if (room === null) return 'here';
  if (entry.roomId === room.roomId) return 'here';
  const requested = entry.requestedRoom ?? null;
  const sameLobby =
    requested !== null && room.catalogId !== undefined && requested.catalogId === room.catalogId;
  if (entry.roomId == null) {
    // 还在进：看它要去哪儿。
    if (requested?.roomId === room.roomId) return 'here';
    return sameLobby && requested.roomId === undefined ? 'here' : 'unrelated';
  }
  return sameLobby ? 'elsewhere' : 'unrelated';
}

/** 对话框打开后房间里新出现的 Agent（网络 Agent 只能这样看到）。 */
export function rosterArrivals(
  present: readonly PresentAgent[],
  baseline: ReadonlySet<string>,
): AgentArrival[] {
  return present
    .filter((agent) => !baseline.has(agent.agentId))
    .map((agent) => ({
      key: `room:${agent.agentId}`,
      agentId: agent.agentId,
      displayName: agent.displayName,
      state: 'ready',
      elsewhere: false,
      errorCode: null,
      active: false,
      existing: false,
    }));
}

/** 同一个 Agent 两边都看到时，只留本机会话那条：它的进度更细。 */
export function mergeArrivals(
  host: readonly AgentArrival[],
  roster: readonly AgentArrival[],
): AgentArrival[] {
  const known = new Set(
    host.flatMap((arrival) => (arrival.agentId === null ? [] : [arrival.agentId])),
  );
  return [...host, ...roster.filter((arrival) => !known.has(arrival.agentId ?? ''))];
}
