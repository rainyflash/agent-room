import { describe, expect, it, vi } from 'vitest';

import { TauriAgentRecoveryGateway } from '@/features/security/adapters/tauri-agent-recovery-gateway';

const sessionId = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const state = {
  userId: '@agent:example.test',
  deviceId: 'DEVICE',
  identity: 'ready',
  recoveryAvailable: true,
  backupEnabled: true,
};

describe('桌面 Agent 恢复边界', () => {
  it('浏览器不调用原生恢复命令', async () => {
    const invoke = vi.fn();
    const gateway = new TauriAgentRecoveryGateway({ available: () => false, invoke });
    expect((await gateway.sessions()).ok).toBe(false);
    expect((await gateway.execute(sessionId, { action: 'inspect' })).ok).toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('恢复凭据的 Unicode、字节与控制字符边界和原生契约一致', async () => {
    const invoke = vi.fn().mockResolvedValue({ state, recoveryKey: 'one-time-test-key' });
    const gateway = new TauriAgentRecoveryGateway({ available: () => true, invoke });
    for (const passphrase of [
      ' '.repeat(12),
      'abc\n'.repeat(4),
      '密'.repeat(342),
      '😀'.repeat(6),
    ]) {
      expect((await gateway.execute(sessionId, { action: 'enable', passphrase })).ok).toBe(false);
    }
    expect(invoke).not.toHaveBeenCalled();
    expect(
      (await gateway.execute(sessionId, { action: 'enable', passphrase: '😀'.repeat(12) })).ok,
    ).toBe(true);
  });

  it('拒绝不合法会话和短口令，不将凭据发往 IPC', async () => {
    const invoke = vi.fn();
    const gateway = new TauriAgentRecoveryGateway({ available: () => true, invoke });
    expect((await gateway.execute('invalid', { action: 'inspect' })).ok).toBe(false);
    expect((await gateway.execute(sessionId, { action: 'enable', passphrase: 'short' })).ok).toBe(
      false,
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it('只接受闭合会话摘要，不向 UI 泄露额外字段', async () => {
    const invoke = vi
      .fn()
      .mockResolvedValue([
        { sessionId, displayName: 'Builder', state: 'ready', credential: 'unexpected' },
      ]);
    const gateway = new TauriAgentRecoveryGateway({ available: () => true, invoke });
    expect(await gateway.sessions()).toEqual({
      ok: false,
      error: { code: 'desktop.agent_recovery.invalid_response', retryable: false },
    });
  });

  it('仅设置操作可返回一次性恢复密钥', async () => {
    const invoke = vi.fn().mockResolvedValue({ state, recoveryKey: 'one-time-test-key' });
    const gateway = new TauriAgentRecoveryGateway({ available: () => true, invoke });
    expect((await gateway.execute(sessionId, { action: 'inspect' })).ok).toBe(false);
    expect(
      (await gateway.execute(sessionId, { action: 'restore', credential: 'test-credential' })).ok,
    ).toBe(false);
    expect(
      (await gateway.execute(sessionId, { action: 'enable', passphrase: 'long-test-passphrase' }))
        .ok,
    ).toBe(true);
    invoke.mockResolvedValue({ state, recoveryKey: null });
    expect(
      (await gateway.execute(sessionId, { action: 'enable', passphrase: 'long-test-passphrase' }))
        .ok,
    ).toBe(false);
  });

  it('保留原生错误码并剔除原始错误中的凭据', async () => {
    const invoke = vi.fn().mockRejectedValue({
      code: 'bridge.security.recovery_rejected',
      retryable: false,
    });
    const gateway = new TauriAgentRecoveryGateway({ available: () => true, invoke });
    expect(await gateway.execute(sessionId, { action: 'restore', credential: 'wrong' })).toEqual({
      ok: false,
      error: { code: 'bridge.security.recovery_rejected', retryable: false },
    });
    invoke.mockRejectedValue({
      code: 'bridge.security.recovery_rejected',
      retryable: false,
      message: 'sensitive credential',
    });
    expect(await gateway.execute(sessionId, { action: 'restore', credential: 'wrong' })).toEqual({
      ok: false,
      error: { code: 'desktop.agent_recovery.failed', retryable: true },
    });
  });
});
