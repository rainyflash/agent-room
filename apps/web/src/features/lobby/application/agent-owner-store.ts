import {
  AGENT_OWNER_LOOKUP_BATCH,
  type AgentOwner,
  type AgentOwnerLookupGateway,
} from '../domain/agent-owners';

const agentIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const noOwners: ReadonlyMap<string, AgentOwner> = new Map();

export type AgentOwnerStoreOptions = {
  readonly now?: () => number;
  /** 攒一小会儿再查，让同一屏里要问的 ID 合成一批。 */
  readonly schedule?: (task: () => void) => void;
  /** 查失败的过多久再问。 */
  readonly retryAfterMs?: number;
  /** 服务器没回答的（此刻还不在房间里，比如刚进来还没确认位置）过多久再问。 */
  readonly unansweredRetryMs?: number;
  /** 过一段时间再做：自己再问一次用。 */
  readonly later?: (task: () => void, delayMs: number) => void;
};

/** 没回答或查失败的，自己最多再问这么多次；之后等名单变了再问。 */
const AUTOMATIC_RETRIES = 3;

type RoomOwners = {
  readonly known: Map<string, AgentOwner>;
  readonly queued: Set<string>;
  readonly inFlight: Set<string>;
  readonly retryAt: Map<string, number>;
  readonly attempts: Map<string, number>;
  scheduled: boolean;
};

/**
 * 按房间记住 Agent 是谁的。服务器只回答此刻在这个房间里的 Agent，所以按房间分开记：回答过的一直记着，
 * 没回答的过一会儿再问。要问的 ID 攒成一批（每批最多 100 个），查失败的过一会儿再问。
 * 退出登录时清空：名字是按登录的人能看的给的。
 */
export class AgentOwnerStore {
  private readonly rooms = new Map<string, RoomOwners>();
  private readonly listeners = new Set<() => void>();
  private snapshot: ReadonlyMap<string, ReadonlyMap<string, AgentOwner>> = new Map();
  private generation = 0;
  private readonly now: () => number;
  private readonly schedule: (task: () => void) => void;
  private readonly retryAfterMs: number;
  private readonly unansweredRetryMs: number;
  private readonly later: (task: () => void, delayMs: number) => void;

  constructor(
    private readonly gateway: AgentOwnerLookupGateway,
    options: AgentOwnerStoreOptions = {},
  ) {
    this.now = options.now ?? Date.now;
    this.schedule = options.schedule ?? ((task) => void setTimeout(task, 30));
    this.retryAfterMs = options.retryAfterMs ?? 60_000;
    this.unansweredRetryMs = options.unansweredRetryMs ?? 30_000;
    this.later = options.later ?? ((task, delayMs) => void setTimeout(task, delayMs));
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** 每个房间问到的 Agent 是谁的；没变的房间拿到的是同一份。 */
  readonly getSnapshot = (): ReadonlyMap<string, ReadonlyMap<string, AgentOwner>> => this.snapshot;

  owners(catalogId: string): ReadonlyMap<string, AgentOwner> {
    return this.snapshot.get(catalogId) ?? noOwners;
  }

  /** 把这个房间里还不知道的 ID 排进下一批；已知的、正在查的、还没到再问时间的都跳过。 */
  request(catalogId: string, agentIds: Iterable<string>): void {
    const room = this.room(catalogId);
    const now = this.now();
    for (const agentId of agentIds) {
      if (
        !agentIdPattern.test(agentId) ||
        room.known.has(agentId) ||
        room.inFlight.has(agentId) ||
        (room.retryAt.get(agentId) ?? 0) > now
      ) {
        continue;
      }
      room.queued.add(agentId);
    }
    if (room.queued.size > 0 && !room.scheduled) {
      room.scheduled = true;
      const generation = this.generation;
      this.schedule(() => {
        if (generation === this.generation) this.flush(catalogId, room);
      });
    }
  }

  /** 退出登录：忘掉所有房间的答案，还在路上的回答也不要了。 */
  clear(): void {
    this.generation += 1;
    this.rooms.clear();
    if (this.snapshot.size === 0) return;
    this.snapshot = new Map();
    for (const listener of this.listeners) listener();
  }

  private room(catalogId: string): RoomOwners {
    let room = this.rooms.get(catalogId);
    if (room === undefined) {
      room = {
        known: new Map(),
        queued: new Set(),
        inFlight: new Set(),
        retryAt: new Map(),
        attempts: new Map(),
        scheduled: false,
      };
      this.rooms.set(catalogId, room);
    }
    return room;
  }

  private flush(catalogId: string, room: RoomOwners): void {
    room.scheduled = false;
    const agentIds = [...room.queued];
    room.queued.clear();
    for (let start = 0; start < agentIds.length; start += AGENT_OWNER_LOOKUP_BATCH) {
      void this.lookup(catalogId, room, agentIds.slice(start, start + AGENT_OWNER_LOOKUP_BATCH));
    }
  }

  private async lookup(catalogId: string, room: RoomOwners, batch: readonly string[]) {
    const generation = this.generation;
    for (const agentId of batch) room.inFlight.add(agentId);
    const result = await this.gateway.lookup(catalogId, batch);
    if (generation !== this.generation) return;
    for (const agentId of batch) room.inFlight.delete(agentId);
    if (!result.ok) {
      this.retryLater(catalogId, room, batch, this.retryAfterMs);
      return;
    }
    const answered = new Map(result.value.map(({ agentId, ...owner }) => [agentId, owner]));
    const unanswered: string[] = [];
    for (const agentId of batch) {
      const owner = answered.get(agentId);
      if (owner === undefined) {
        unanswered.push(agentId);
        continue;
      }
      room.known.set(agentId, owner);
      room.retryAt.delete(agentId);
      room.attempts.delete(agentId);
    }
    this.retryLater(catalogId, room, unanswered, this.unansweredRetryMs);
    if (unanswered.length === batch.length) return;
    this.snapshot = new Map([...this.snapshot, [catalogId, new Map(room.known)]]);
    for (const listener of this.listeners) listener();
  }

  /** 到时候之前不再问；还没自己问够次数的，到时候自己再问一次。 */
  private retryLater(
    catalogId: string,
    room: RoomOwners,
    agentIds: readonly string[],
    delayMs: number,
  ): void {
    const retryAt = this.now() + delayMs;
    const again: string[] = [];
    for (const agentId of agentIds) {
      room.retryAt.set(agentId, retryAt);
      const attempts = (room.attempts.get(agentId) ?? 0) + 1;
      room.attempts.set(agentId, attempts);
      if (attempts <= AUTOMATIC_RETRIES) again.push(agentId);
    }
    if (again.length === 0) return;
    const generation = this.generation;
    this.later(() => {
      if (generation === this.generation) this.request(catalogId, again);
    }, delayMs);
  }
}
