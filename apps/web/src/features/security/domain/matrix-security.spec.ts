import { describe, expect, it } from 'vitest';

import {
  evaluateMatrixSecurity,
  isValidRecoveryPassphrase,
  type MatrixSecurityEvidence,
} from '@/features/security/domain/matrix-security';

describe('evaluateMatrixSecurity', () => {
  it('只在当前设备已验证、恢复链完整且目标房间加密时给出就绪状态', () => {
    expect(evaluateMatrixSecurity(evidence())).toEqual({
      blockers: [],
      excludedDeviceCount: 0,
      kind: 'ready',
      sendAllowed: true,
    });
  });

  it('备份不可恢复时提示操作，但不会把已加密的新消息降级为不可发送', () => {
    expect(
      evaluateMatrixSecurity(evidence({ backup: 'locked', secretStorageReady: false })),
    ).toEqual({
      blockers: ['secret_storage_missing', 'backup_locked'],
      excludedDeviceCount: 0,
      kind: 'action_required',
      sendAllowed: true,
    });
  });

  it('未签名当前设备或未加密房间必须失败关闭，并统计被排除设备', () => {
    const base = evidence();
    expect(
      evaluateMatrixSecurity({
        ...base,
        devices: [
          ...base.devices.map((device) => ({ ...device, trust: 'unverified' as const })),
          {
            current: false,
            deviceId: 'BOB-OLD',
            trust: 'unverified',
            userId: '@bob:agent-room.test',
          },
        ],
        roomEncryption: 'unencrypted',
      }),
    ).toEqual({
      blockers: ['current_device_unverified', 'room_unencrypted'],
      excludedDeviceCount: 1,
      kind: 'blocked',
      sendAllowed: false,
    });
  });
});

describe('isValidRecoveryPassphrase', () => {
  it('拒绝短口令、纯空白和异常长输入', () => {
    expect(isValidRecoveryPassphrase('short')).toBe(false);
    expect(isValidRecoveryPassphrase(' '.repeat(12))).toBe(false);
    expect(isValidRecoveryPassphrase('x'.repeat(257))).toBe(false);
    expect(isValidRecoveryPassphrase('correct horse battery staple')).toBe(true);
  });
});

function evidence(overrides: Partial<MatrixSecurityEvidence> = {}): MatrixSecurityEvidence {
  return {
    backup: 'ready',
    crossSigningReady: true,
    cryptoVersion: 'Rust SDK test',
    currentDeviceId: 'ALICE-WEB',
    devices: [
      {
        current: true,
        deviceId: 'ALICE-WEB',
        trust: 'verified',
        userId: '@alice:agent-room.test',
      },
    ],
    roomEncryption: 'encrypted',
    roomId: '!private:agent-room.test',
    secretStorageReady: true,
    userId: '@alice:agent-room.test',
    ...overrides,
  };
}
