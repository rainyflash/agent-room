import type { LobbyAgent, LobbyGateway, LobbyRoom } from '@/features/lobby/domain/lobby';

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
/**
 * 房间除了观察时刻都没变时，这么久之内沿用上一份快照、不通知订阅者。
 *
 * 每次 Matrix 同步和每 5 秒的过期时钟都会重读房间，以前每次都换一份新对象，整个大厅跟着重绘。
 * 在线、重连、等待这些随时间变化的状态由网关算进每个 Agent，变了照常立刻通知；只有按小时分组的
 * 离线时长靠观察时刻推进，一分钟更新一次足够。
 */
export const LOBBY_IDLE_REFRESH_MS = 60_000;

export class LobbyRoomStore {
  readonly #gateway: LobbyGateway;
  readonly #listeners = new Set<() => void>();
  readonly #roomId: string;
  #detachGateway: (() => void) | null = null;
  #expiryTimer: ReturnType<typeof setInterval> | null = null;
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
      // Expiration must also advance in a quiet room with no Matrix events.
      this.#expiryTimer = setInterval(this.#refresh, 5_000);
    }
    return () => {
      this.#listeners.delete(listener);
      if (this.#listeners.size === 0) {
        this.#detachGateway?.();
        this.#detachGateway = null;
        if (this.#expiryTimer !== null) clearInterval(this.#expiryTimer);
        this.#expiryTimer = null;
      }
    };
  };

  readonly #refresh = (): void => {
    const result = this.#gateway.read(this.#roomId);
    const next: LobbyRoomState = result.ok
      ? Object.freeze({ kind: 'ready', room: result.value })
      : Object.freeze({
          code: result.error.code,
          kind: 'failed',
          retryable: result.error.retryable,
        });
    const settled = settle(this.#state, next);
    if (settled === this.#state) return;
    this.#state = settled;
    for (const listener of this.#listeners) {
      listener();
    }
  };
}

/** 没变就沿用上一份；变了也尽量沿用没变的 Agent 对象，让界面只重绘变了的部分。 */
function settle(previous: LobbyRoomState, next: LobbyRoomState): LobbyRoomState {
  if (previous.kind === 'ready' && next.kind === 'ready') {
    const room = shareRoom(previous.room, next.room);
    return room === previous.room ? previous : Object.freeze({ kind: 'ready', room });
  }
  return sameData(previous, next) ? previous : next;
}

function shareRoom(previous: LobbyRoom, next: LobbyRoom): LobbyRoom {
  const agents = shareAgents(previous.agents, next.agents);
  if (
    agents === previous.agents &&
    next.observedAtUnixMs - previous.observedAtUnixMs < LOBBY_IDLE_REFRESH_MS &&
    sameData(
      { ...previous, agents: null, observedAtUnixMs: 0 },
      { ...next, agents: null, observedAtUnixMs: 0 },
    )
  ) {
    return previous;
  }
  return Object.freeze({ ...next, agents });
}

function shareAgents(
  previous: readonly LobbyAgent[],
  next: readonly LobbyAgent[],
): readonly LobbyAgent[] {
  const byId = new Map(previous.map((agent) => [agent.agentId, agent]));
  let changed = previous.length !== next.length;
  const shared = next.map((agent, index) => {
    const earlier = byId.get(agent.agentId);
    const kept = earlier !== undefined && sameData(earlier, agent) ? earlier : agent;
    if (kept !== previous[index]) changed = true;
    return kept;
  });
  return changed ? Object.freeze(shared) : previous;
}

/** 只比较房间投影这种纯数据：对象、数组和原始值。 */
function sameData(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) {
    return false;
  }
  if (Array.isArray(left) || Array.isArray(right)) {
    return (
      Array.isArray(left) &&
      Array.isArray(right) &&
      left.length === right.length &&
      left.every((item, index) => sameData(item, right[index]))
    );
  }
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const keys = Object.keys(leftRecord);
  return (
    keys.length === Object.keys(rightRecord).length &&
    keys.every(
      (key) => Object.hasOwn(rightRecord, key) && sameData(leftRecord[key], rightRecord[key]),
    )
  );
}
