import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneAccessManagementClient } from './control-plane-access-management-client';

const DEVICE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const INSTANCE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';

const DEVICE = {
  createdAtUnixMs: 1_700_000_000_000,
  deviceId: DEVICE_ID,
  label: 'Studio workstation',
  lastSeenAtUnixMs: 1_700_000_010_000,
  matrixDeviceId: 'WEB_DEVICE',
  platform: 'windows',
  revokedAtUnixMs: null,
  trustState: 'verified',
} as const;

const INSTANCE = {
  adapterType: 'codex',
  agentAvatarContentId: null,
  agentDisplayName: 'Build agent',
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  agentInstanceId: INSTANCE_ID,
  capabilityVersion: '1.0',
  createdAtUnixMs: 1_700_000_000_000,
  device: {
    deviceId: DEVICE_ID,
    label: 'Studio workstation',
    platform: 'windows',
    trustState: 'verified',
  },
  lastSeenAtUnixMs: 1_700_000_010_000,
  matrixDeviceId: 'AR_INSTANCE',
  matrixDeviceRevokedAtUnixMs: null,
  revokedAtUnixMs: null,
  status: 'online',
} as const;

describe('ControlPlaneAccessManagementClient', () => {
  it('分别校验产品设备和 Agent 实例的权威列表', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(Response.json({ devices: [DEVICE] }))
      .mockResolvedValueOnce(Response.json({ instances: [INSTANCE] }));
    const client = createClient(fetch);

    const devices = await client.listProductDevices();
    const instances = await client.listAgentInstances();

    expect(devices.ok ? devices.value[0]?.label : null).toBe('Studio workstation');
    expect(instances.ok ? instances.value[0]?.agentDisplayName : null).toBe('Build agent');
    expect(fetch).toHaveBeenNthCalledWith(
      1,
      new URL('https://control.agent-room.test/auth/devices'),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('把设备撤销的 204 和 202 映射为显式清理状态', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        Response.json(
          {
            localRevocation: 'complete',
            matrixCleanup: 'pending',
            pendingAgentInstanceCount: 2,
          },
          { status: 202 },
        ),
      );
    const client = createClient(fetch);

    expect(await client.revokeProductDevice(DEVICE_ID)).toEqual({
      ok: true,
      value: { matrixCleanup: 'complete', pendingAgentInstanceCount: 0 },
    });
    expect(await client.revokeProductDevice(DEVICE_ID)).toEqual({
      ok: true,
      value: { matrixCleanup: 'pending', pendingAgentInstanceCount: 2 },
    });
  });

  it('保留 Agent 实例撤销的远端清理失败原因', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      Response.json(
        {
          instance: { ...INSTANCE, revokedAtUnixMs: 1_700_000_020_000, status: 'revoked' },
          matrixCleanup: 'pending',
          matrixCleanupPendingReason: 'dependencyUnavailable',
        },
        { status: 202 },
      ),
    );

    const result = await createClient(fetch).revokeAgentInstance(INSTANCE_ID);

    expect(result.ok ? result.value.matrixCleanup : null).toBe('pending');
    expect(result.ok ? result.value.matrixCleanupPendingReason : null).toBe(
      'dependencyUnavailable',
    );
  });

  it('拒绝结构损坏的响应而不是让未知状态进入界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ devices: [{ ...DEVICE, trustState: 'god-mode' }] }));

    const result = await createClient(fetch).listProductDevices();

    expect(result).toEqual({
      error: { code: 'access.invalid_device_list', retryable: false },
      ok: false,
    });
  });
});

function createClient(fetch: typeof globalThis.fetch) {
  return new ControlPlaneAccessManagementClient({
    baseUrl: 'https://control.agent-room.test',
    fetch,
  });
}
