import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';

const DAY_MS = 24 * 60 * 60 * 1_000;
/** 这么久以内用过的电脑摊开列着，更久没用的收起来（`specs/workspace-tidy/design.md`）。 */
export const RECENT_DEVICE_MS = 7 * DAY_MS;
/** 这台电脑的授权连续这么久不用就过期，要重新授权才能用（`specs/session-lifetime/design.md`）。 */
export const DEVICE_AUTHORIZATION_IDLE_MS = 30 * DAY_MS;

export type DeviceGroups = {
  /** 最近 7 天用过的，或者还没用够 7 天的新电脑。 */
  readonly recent: readonly ProductDevice[];
  /** 7 天以上没用的，“全部撤销”只撤这一组。 */
  readonly idle: readonly ProductDevice[];
  readonly revoked: readonly ProductDevice[];
};

export type InstanceGroups = {
  /** 连着的：在线、正在连接、连接不稳。 */
  readonly connected: readonly AgentInstance[];
  readonly offline: readonly AgentInstance[];
  /** 已经断开（撤销）的。 */
  readonly revoked: readonly AgentInstance[];
};

/**
 * 这台电脑上次用的时候：Bridge 换令牌时记的“上次使用”，从没记过的按注册的时候算。
 */
export function deviceLastUsedAt(device: ProductDevice): number {
  return device.lastSeenAtUnixMs ?? device.createdAtUnixMs;
}

/** 授权已经过期了：连续 30 天没用。撤销了的不算。 */
export function deviceAuthorizationExpired(device: ProductDevice, now: number): boolean {
  return (
    device.trustState !== 'revoked' && now - deviceLastUsedAt(device) > DEVICE_AUTHORIZATION_IDLE_MS
  );
}

/** 按上次使用分组，每组里最近用过的在前。 */
export function groupDevices(devices: readonly ProductDevice[], now: number): DeviceGroups {
  const sorted = [...devices].sort(
    (left, right) => deviceLastUsedAt(right) - deviceLastUsedAt(left),
  );
  return {
    recent: sorted.filter(
      (device) =>
        device.trustState !== 'revoked' && now - deviceLastUsedAt(device) <= RECENT_DEVICE_MS,
    ),
    idle: sorted.filter(
      (device) =>
        device.trustState !== 'revoked' && now - deviceLastUsedAt(device) > RECENT_DEVICE_MS,
    ),
    revoked: sorted.filter((device) => device.trustState === 'revoked'),
  };
}

/** 按连没连着分组，每组里最近活动过的在前。 */
export function groupInstances(instances: readonly AgentInstance[]): InstanceGroups {
  const lastActive = (instance: AgentInstance): number =>
    instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs;
  const sorted = [...instances].sort((left, right) => lastActive(right) - lastActive(left));
  return {
    connected: sorted.filter(
      (instance) => instance.status !== 'offline' && instance.status !== 'revoked',
    ),
    offline: sorted.filter((instance) => instance.status === 'offline'),
    revoked: sorted.filter((instance) => instance.status === 'revoked'),
  };
}
