import type { FleetAgent, FleetInstance } from './agent-fleet';

/** 卡片上的一行状态：在线、正在连、连接不稳，或者上次在线的时间，从没连上过的也说清。 */
export type AgentActivity =
  | { readonly kind: 'online' | 'connecting' | 'degraded' }
  | { readonly kind: 'lastSeen'; readonly atUnixMs: number }
  | { readonly kind: 'never' };

/** 它在哪儿运行：这台电脑、别的某台设备、几台设备，或者是网络 Agent。 */
export type AgentPlace =
  | { readonly kind: 'network' }
  | { readonly kind: 'thisComputer' }
  | { readonly kind: 'device'; readonly label: string }
  | { readonly kind: 'devices'; readonly count: number }
  | { readonly kind: 'none' };

/** 服务器替网络 Agent 登记的连接用这个适配器类型（ADR 0010）。 */
const NETWORK_ADAPTER = 'network';

export function agentActivity(entry: FleetAgent): AgentActivity {
  return statusActivity(entry.status, entry.lastSeenAtUnixMs);
}

export function instanceActivity(instance: FleetInstance): AgentActivity {
  return statusActivity(instance.status, instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs);
}

function statusActivity(
  status: FleetAgent['status'],
  lastSeenAtUnixMs: number | null,
): AgentActivity {
  switch (status) {
    case 'online':
    case 'connecting':
    case 'degraded':
      return { kind: status };
    case 'offline':
    case 'revoked':
      return lastSeenAtUnixMs === null
        ? { kind: 'never' }
        : { kind: 'lastSeen', atUnixMs: lastSeenAtUnixMs };
  }
}

/** 撤销了的连接不算；这台电脑上有一份就说“这台电脑”，它最相关。 */
export function agentPlace(entry: FleetAgent): AgentPlace {
  const live = liveInstances(entry);
  const first = live[0];
  if (first === undefined) return { kind: 'none' };
  if (live.some((instance) => instance.adapterType === NETWORK_ADAPTER)) return { kind: 'network' };
  if (live.some((instance) => instance.currentDevice)) return { kind: 'thisComputer' };
  const devices = new Set(live.map((instance) => instance.device.deviceId));
  return devices.size === 1
    ? { kind: 'device', label: first.device.label }
    : { kind: 'devices', count: devices.size };
}

export function liveInstances(entry: FleetAgent): readonly FleetInstance[] {
  return entry.instances.filter((instance) => instance.revokedAtUnixMs === null);
}

export function isNetworkInstance(instance: FleetInstance): boolean {
  return instance.adapterType === NETWORK_ADAPTER;
}
