import {
  NETWORK_AGENT_LOOKUP_BATCH,
  type NetworkAgentLookupGateway,
} from '../domain/network-agent-labels';

const agentIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const empty: ReadonlySet<string> = new Set();

export type NetworkAgentLabelStoreOptions = {
  readonly now?: () => number;
  /** 攒一小会儿再查，让同一屏里要问的 ID 合成一批。 */
  readonly schedule?: (task: () => void) => void;
  /** 查失败的 ID 过多久再问。 */
  readonly retryAfterMs?: number;
};

/**
 * 记住哪些 Agent 是网络 Agent。答案对同一个 Agent 不会变，查过的一直记着；
 * 要问的 ID 攒成一批（每批最多 100 个），查失败的过一会儿再问，不会反复打扰服务器。
 */
export class NetworkAgentLabelStore {
  private readonly known = new Map<string, boolean>();
  private readonly queued = new Set<string>();
  private readonly inFlight = new Set<string>();
  private readonly retryAt = new Map<string, number>();
  private readonly listeners = new Set<() => void>();
  private snapshot: ReadonlySet<string> = empty;
  private scheduled = false;
  private readonly now: () => number;
  private readonly schedule: (task: () => void) => void;
  private readonly retryAfterMs: number;

  constructor(
    private readonly gateway: NetworkAgentLookupGateway,
    options: NetworkAgentLabelStoreOptions = {},
  ) {
    this.now = options.now ?? Date.now;
    this.schedule = options.schedule ?? ((task) => void setTimeout(task, 30));
    this.retryAfterMs = options.retryAfterMs ?? 60_000;
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** 已知是网络 Agent 的 ID。 */
  readonly getSnapshot = (): ReadonlySet<string> => this.snapshot;

  /** 把还不知道的 ID 排进下一批；已知的、正在查的、刚查失败的都跳过。 */
  request(agentIds: Iterable<string>): void {
    const now = this.now();
    for (const agentId of agentIds) {
      if (
        !agentIdPattern.test(agentId) ||
        this.known.has(agentId) ||
        this.inFlight.has(agentId) ||
        (this.retryAt.get(agentId) ?? 0) > now
      ) {
        continue;
      }
      this.queued.add(agentId);
    }
    if (this.queued.size > 0 && !this.scheduled) {
      this.scheduled = true;
      this.schedule(() => {
        this.flush();
      });
    }
  }

  private flush(): void {
    this.scheduled = false;
    const agentIds = [...this.queued];
    this.queued.clear();
    for (let start = 0; start < agentIds.length; start += NETWORK_AGENT_LOOKUP_BATCH) {
      void this.lookup(agentIds.slice(start, start + NETWORK_AGENT_LOOKUP_BATCH));
    }
  }

  private async lookup(batch: readonly string[]): Promise<void> {
    for (const agentId of batch) this.inFlight.add(agentId);
    const result = await this.gateway.lookup(batch);
    for (const agentId of batch) this.inFlight.delete(agentId);
    if (!result.ok) {
      const retryAt = this.now() + this.retryAfterMs;
      for (const agentId of batch) this.retryAt.set(agentId, retryAt);
      return;
    }
    let found = false;
    for (const agentId of batch) {
      const network = result.value.has(agentId);
      this.known.set(agentId, network);
      this.retryAt.delete(agentId);
      found ||= network;
    }
    if (!found) return;
    this.snapshot = new Set(
      [...this.known].filter(([, network]) => network).map(([agentId]) => agentId),
    );
    for (const listener of this.listeners) listener();
  }
}
