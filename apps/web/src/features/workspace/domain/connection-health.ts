import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import type { AgentFleet, FleetInstanceStatus } from '@/features/workspace/domain/agent-fleet';

export type WorkspaceLayerId = 'agents' | 'bridge' | 'controlPlane' | 'matrix';
export type WorkspaceLayerStatus = FleetInstanceStatus | 'unavailable';

export type WorkspaceLayerHealth = {
  readonly failureCode: string | null;
  readonly observedAtUnixMs: number | null;
  readonly status: WorkspaceLayerStatus;
};

export type WorkspaceConnectionHealth = Readonly<Record<WorkspaceLayerId, WorkspaceLayerHealth>>;

export type WorkspaceProbeResult = { readonly ok: boolean } | undefined;

export type WorkspaceConnectionHealthInput = {
  readonly agents: {
    readonly failureCode: string | null;
    readonly fleet: AgentFleet;
    readonly loading: boolean;
  };
  readonly bridge: {
    readonly available: boolean;
    readonly changedAtUnixMs: number | null;
    readonly failureCode: string | null;
    readonly phase: BridgePhase | undefined;
  };
  readonly controlPlane: {
    readonly failureCode: string | null;
    readonly observedAtUnixMs: number | null;
    readonly pending: boolean;
    readonly results: readonly WorkspaceProbeResult[];
  };
  readonly matrix: {
    readonly failureCode: string | null;
    readonly observedAtUnixMs: number | null;
    readonly pending: boolean;
    readonly result: WorkspaceProbeResult;
  };
};

export function projectWorkspaceConnectionHealth(
  input: WorkspaceConnectionHealthInput,
): WorkspaceConnectionHealth {
  return Object.freeze({
    agents: projectAgentHealth(input.agents),
    bridge: Object.freeze({
      failureCode: input.bridge.failureCode,
      observedAtUnixMs: input.bridge.changedAtUnixMs,
      status: bridgeWorkspaceStatus(input.bridge.available, input.bridge.phase),
    }),
    controlPlane: projectRemoteHealth({
      failureCode: input.controlPlane.failureCode,
      observedAtUnixMs: input.controlPlane.observedAtUnixMs,
      pending: input.controlPlane.pending,
      results: input.controlPlane.results,
    }),
    matrix: projectRemoteHealth({
      failureCode: input.matrix.failureCode,
      observedAtUnixMs: input.matrix.observedAtUnixMs,
      pending: input.matrix.pending,
      results: [input.matrix.result],
    }),
  });
}

export function bridgeWorkspaceStatus(
  available: boolean,
  phase: BridgePhase | undefined,
): WorkspaceLayerStatus {
  if (!available) return 'unavailable';
  const statuses: Readonly<Record<BridgePhase, WorkspaceLayerStatus>> = {
    authorization_required: 'connecting',
    authorized: 'connecting',
    discovering: 'connecting',
    halted: 'degraded',
    ready: 'online',
    retry_scheduled: 'degraded',
    starting: 'connecting',
    stopped: 'offline',
  };
  return phase === undefined ? 'connecting' : statuses[phase];
}

function projectRemoteHealth(input: {
  readonly failureCode: string | null;
  readonly observedAtUnixMs: number | null;
  readonly pending: boolean;
  readonly results: readonly WorkspaceProbeResult[];
}): WorkspaceLayerHealth {
  const settled = input.results.filter(
    (result): result is Exclude<WorkspaceProbeResult, undefined> => result !== undefined,
  );
  const successful = settled.filter((result) => result.ok).length;
  return Object.freeze({
    failureCode: input.failureCode,
    observedAtUnixMs: input.observedAtUnixMs,
    status: remoteStatus(settled.length, successful, input.pending),
  });
}

function remoteStatus(
  settledCount: number,
  successfulCount: number,
  pending: boolean,
): WorkspaceLayerStatus {
  if (settledCount === 0) return pending ? 'connecting' : 'offline';
  if (successfulCount === settledCount) return 'online';
  if (successfulCount === 0) return 'offline';
  return 'degraded';
}

function projectAgentHealth(input: {
  readonly failureCode: string | null;
  readonly fleet: AgentFleet;
  readonly loading: boolean;
}): WorkspaceLayerHealth {
  const instances = input.fleet.agents.flatMap((agent) => agent.instances);
  const observedAtUnixMs = instances.reduce<number | null>((latest, instance) => {
    const observed = instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs;
    return latest === null || observed > latest ? observed : latest;
  }, null);
  return Object.freeze({
    failureCode: input.failureCode,
    observedAtUnixMs,
    status: aggregateAgentStatus(
      instances.map((instance) => instance.status),
      input.loading,
      input.failureCode,
    ),
  });
}

function aggregateAgentStatus(
  statuses: readonly FleetInstanceStatus[],
  loading: boolean,
  failureCode: string | null,
): WorkspaceLayerStatus {
  if (loading) return 'connecting';
  if (failureCode !== null) return 'degraded';
  if (statuses.length === 0) return 'offline';
  if (statuses.every((status) => status === 'revoked')) return 'revoked';
  if (statuses.every((status) => status === 'online')) return 'online';
  if (statuses.every((status) => status === 'offline' || status === 'revoked')) return 'offline';
  if (statuses.includes('connecting') && !statuses.includes('online')) return 'connecting';
  return 'degraded';
}
