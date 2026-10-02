import { describe, expect, it } from 'vitest';

import { thisDeviceState } from '@/features/security/domain/device-signing';
import type {
  MatrixDeviceTrust,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';

describe('thisDeviceState', () => {
  it('由你签过名就算就绪，不管自动签名记的是什么', () => {
    for (const trust of ['signed', 'verified'] as const) {
      expect(thisDeviceState(snapshot(trust), { kind: 'failed' })).toBe('ready');
      expect(thisDeviceState(snapshot(trust), { kind: 'working' })).toBe('ready');
    }
  });

  it('自动签名刚跑完、设备列表还没刷新到时也算就绪', () => {
    expect(thisDeviceState(snapshot('unverified'), { kind: 'ready' })).toBe('ready');
    expect(thisDeviceState(null, { kind: 'ready' })).toBe('ready');
  });

  it('还没签名时看自动签名：出错就给重试，其余都是正在准备', () => {
    expect(thisDeviceState(snapshot('unverified'), { kind: 'failed' })).toBe('failed');
    expect(thisDeviceState(snapshot('unknown'), { kind: 'working' })).toBe('working');
    expect(thisDeviceState(snapshot('unverified'), { kind: 'idle' })).toBe('working');
    expect(thisDeviceState(null, { kind: 'failed' })).toBe('failed');
  });
});

function snapshot(trust: MatrixDeviceTrust): Pick<MatrixSecuritySnapshot, 'devices'> {
  return {
    devices: [
      { current: true, deviceId: 'WEB', trust, userId: '@alice:agent-room.test' },
      { current: false, deviceId: 'DESKTOP', trust: 'signed', userId: '@alice:agent-room.test' },
    ],
  };
}
