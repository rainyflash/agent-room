import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';

export type FleetInstanceStatus = AgentInstance['status'];

export type FleetInstance = AgentInstance & {
  readonly currentDevice: boolean;
};

export type FleetAgent = {
  readonly agent: OwnedAgent;
  readonly instances: readonly FleetInstance[];
  readonly lastSeenAtUnixMs: number | null;
  readonly status: FleetInstanceStatus;
};

export type FleetDevice = ProductDevice & {
  readonly current: boolean;
  readonly instanceCount: number;
};

export type AgentFleet = {
  readonly agents: readonly FleetAgent[];
  readonly devices: readonly FleetDevice[];
  readonly orphanInstances: readonly AgentInstance[];
};

const statusPriority: Readonly<Record<FleetInstanceStatus, number>> = Object.freeze({
  connecting: 4,
  degraded: 3,
  offline: 2,
  online: 5,
  revoked: 1,
});

export function projectAgentFleet(input: {
  readonly agents: readonly OwnedAgent[];
  readonly currentMatrixDeviceId: string | null;
  readonly devices: readonly ProductDevice[];
  readonly instances: readonly AgentInstance[];
}): AgentFleet {
  const agentsById = new Map(input.agents.map((agent) => [agent.agentId, agent]));
  const currentDeviceIds = new Set(
    input.devices
      .filter((device) => device.matrixDeviceId === input.currentMatrixDeviceId)
      .map((device) => device.deviceId),
  );
  const instancesByAgent = new Map<string, FleetInstance[]>();
  const instanceCountsByDevice = new Map<string, number>();
  const orphanInstances: AgentInstance[] = [];

  for (const instance of input.instances) {
    if (!agentsById.has(instance.agentId)) {
      orphanInstances.push(instance);
      continue;
    }
    const projected: FleetInstance = {
      ...instance,
      currentDevice: currentDeviceIds.has(instance.device.deviceId),
    };
    const agentInstances = instancesByAgent.get(instance.agentId) ?? [];
    agentInstances.push(projected);
    instancesByAgent.set(instance.agentId, agentInstances);
    instanceCountsByDevice.set(
      instance.device.deviceId,
      (instanceCountsByDevice.get(instance.device.deviceId) ?? 0) + 1,
    );
  }

  const agents = input.agents
    .map((agent): FleetAgent => {
      const instances = [...(instancesByAgent.get(agent.agentId) ?? [])].sort(compareInstances);
      return {
        agent,
        instances: Object.freeze(instances),
        lastSeenAtUnixMs: latestTimestamp(instances),
        status: aggregateStatus(instances),
      };
    })
    .sort(compareAgents);

  const devices = input.devices
    .map((device): FleetDevice => ({
      ...device,
      current: currentDeviceIds.has(device.deviceId),
      instanceCount: instanceCountsByDevice.get(device.deviceId) ?? 0,
    }))
    .sort(compareDevices);

  return Object.freeze({
    agents: Object.freeze(agents),
    devices: Object.freeze(devices),
    orphanInstances: Object.freeze(orphanInstances.sort(compareInstances)),
  });
}

function aggregateStatus(instances: readonly FleetInstance[]): FleetInstanceStatus {
  if (instances.length === 0) return 'offline';
  return instances.reduce(
    (highest, instance) =>
      statusPriority[instance.status] > statusPriority[highest] ? instance.status : highest,
    instances[0]?.status ?? 'offline',
  );
}

function latestTimestamp(instances: readonly FleetInstance[]): number | null {
  return instances.reduce<number | null>((latest, instance) => {
    const observed = instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs;
    return latest === null || observed > latest ? observed : latest;
  }, null);
}

function compareAgents(left: FleetAgent, right: FleetAgent): number {
  const priority = statusPriority[right.status] - statusPriority[left.status];
  return priority !== 0
    ? priority
    : left.agent.displayName.localeCompare(right.agent.displayName, undefined, {
        sensitivity: 'base',
      });
}

function compareInstances(left: AgentInstance, right: AgentInstance): number {
  const priority = statusPriority[right.status] - statusPriority[left.status];
  if (priority !== 0) return priority;
  const leftSeen = left.lastSeenAtUnixMs ?? left.createdAtUnixMs;
  const rightSeen = right.lastSeenAtUnixMs ?? right.createdAtUnixMs;
  return rightSeen - leftSeen;
}

function compareDevices(left: FleetDevice, right: FleetDevice): number {
  if (left.current !== right.current) return left.current ? -1 : 1;
  const leftRevoked = left.revokedAtUnixMs !== null;
  const rightRevoked = right.revokedAtUnixMs !== null;
  if (leftRevoked !== rightRevoked) return leftRevoked ? 1 : -1;
  return left.label.localeCompare(right.label, undefined, { sensitivity: 'base' });
}
