import type { HandoffTarget, HandoffTargetDevice } from '@/features/handoffs/domain/handoff';

export type HandoffTargetDeviceGroup = {
  readonly device: HandoffTargetDevice;
  readonly targets: readonly HandoffTarget[];
};

export type HandoffTargetAgentGroup = {
  readonly agentAvatarContentId: string | null;
  readonly agentDisplayName: string;
  readonly agentId: string;
  readonly devices: readonly HandoffTargetDeviceGroup[];
};

type MutableDeviceGroup = {
  readonly device: HandoffTargetDevice;
  readonly targets: HandoffTarget[];
};

type MutableAgentGroup = {
  readonly agentAvatarContentId: string | null;
  readonly agentDisplayName: string;
  readonly agentId: string;
  readonly devices: Map<string, MutableDeviceGroup>;
};

export function groupHandoffTargets(
  targets: readonly HandoffTarget[],
): readonly HandoffTargetAgentGroup[] {
  const agents = new Map<string, MutableAgentGroup>();

  for (const target of targets) {
    const agent: MutableAgentGroup = agents.get(target.agentId) ?? {
      agentAvatarContentId: target.agentAvatarContentId,
      agentDisplayName: target.agentDisplayName,
      agentId: target.agentId,
      devices: new Map<string, MutableDeviceGroup>(),
    };
    const device: MutableDeviceGroup = agent.devices.get(target.device.deviceId) ?? {
      device: target.device,
      targets: [],
    };
    device.targets.push(target);
    agent.devices.set(target.device.deviceId, device);
    agents.set(target.agentId, agent);
  }

  return Object.freeze(
    [...agents.values()].sort(compareAgents).map((agent) =>
      Object.freeze({
        agentAvatarContentId: agent.agentAvatarContentId,
        agentDisplayName: agent.agentDisplayName,
        agentId: agent.agentId,
        devices: Object.freeze(
          [...agent.devices.values()].sort(compareDevices).map((device) =>
            Object.freeze({
              device: Object.freeze(device.device),
              targets: Object.freeze([...device.targets].sort(compareTargets)),
            }),
          ),
        ),
      }),
    ),
  );
}

function compareAgents(
  left: { readonly agentDisplayName: string; readonly agentId: string },
  right: { readonly agentDisplayName: string; readonly agentId: string },
): number {
  return (
    compareText(left.agentDisplayName, right.agentDisplayName) ||
    compareText(left.agentId, right.agentId)
  );
}

function compareDevices(
  left: { readonly device: HandoffTargetDevice; readonly targets: readonly HandoffTarget[] },
  right: { readonly device: HandoffTargetDevice; readonly targets: readonly HandoffTarget[] },
): number {
  const availability =
    Number(hasOnlineTarget(right.targets)) - Number(hasOnlineTarget(left.targets));
  return (
    availability ||
    compareText(left.device.label, right.device.label) ||
    compareText(left.device.deviceId, right.device.deviceId)
  );
}

function compareTargets(left: HandoffTarget, right: HandoffTarget): number {
  const availability = Number(right.online) - Number(left.online);
  return (
    availability ||
    compareText(left.adapterType, right.adapterType) ||
    compareText(left.instanceId, right.instanceId)
  );
}

function hasOnlineTarget(targets: readonly HandoffTarget[]): boolean {
  return targets.some((target) => target.online);
}

function compareText(left: string, right: string): number {
  return left.localeCompare(right, 'en', { sensitivity: 'base' });
}
