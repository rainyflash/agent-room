// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';

import { BrowserMatrixSessionVault } from './browser-matrix-session-vault';
import { TauriMatrixSessionVault, type MatrixVaultInvoke } from './tauri-matrix-session-vault';
import {
  storedMatrixSessionSchema,
  type StoredMatrixSession,
} from '../domain/matrix-session-vault';
import { ok } from '@/shared/result';

const session: StoredMatrixSession = {
  accessToken: 'test-access',
  deviceId: 'TEST_DEVICE',
  refreshToken: 'test-refresh',
  userId: '@tester:matrix.test',
  version: 1,
};

describe('Matrix 会话存储适配器', () => {
  beforeEach(() => {
    sessionStorage.clear();
    localStorage.clear();
  });

  it('网页保持标签页存储并支持旧版会话记录', async () => {
    const vault = new BrowserMatrixSessionVault(sessionStorage);
    await expect(vault.load()).resolves.toEqual(ok(null));
    await vault.save(session);
    expect(sessionStorage.getItem('agent-room.matrix-session.v1')).toBe(JSON.stringify(session));
    expect(localStorage.length).toBe(0);
    await expect(new BrowserMatrixSessionVault(sessionStorage).load()).resolves.toEqual(
      ok(session),
    );
    await vault.clear();
    await expect(vault.load()).resolves.toEqual(ok(null));
  });

  it('桌面仅调用三条固定命令而不把令牌写入网页存储', async () => {
    const call = vi.fn<MatrixVaultInvoke>().mockResolvedValueOnce(session).mockResolvedValue(null);
    const vault = new TauriMatrixSessionVault(call);
    await expect(vault.load()).resolves.toEqual(ok(session));
    await expect(vault.save(session)).resolves.toEqual(ok(undefined));
    await expect(vault.clear()).resolves.toEqual(ok(undefined));
    expect(call.mock.calls).toEqual([
      ['desktop_load_matrix_session', {}],
      ['desktop_save_matrix_session', { session }],
      ['desktop_clear_matrix_session', {}],
    ]);
    expect(sessionStorage.length).toBe(0);
    expect(localStorage.length).toBe(0);
  });

  it.each([
    {},
    { ...session, version: 2 },
    { ...session, userId: 'not-matrix' },
    { ...session, accessToken: 'contains\nnewline' },
    { ...session, extra: true },
  ])('拒绝损坏和越界的凭据响应 %#', async (value) => {
    expect(storedMatrixSessionSchema.safeParse(value).success).toBe(false);
    const vault = new TauriMatrixSessionVault(vi.fn<MatrixVaultInvoke>().mockResolvedValue(value));
    await expect(vault.load()).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.matrix_session.vault_invalid_response' },
    });
  });

  it.each([
    { code: 'desktop.matrix_session.vault_corrupt', retryable: false },
    JSON.stringify({ code: 'desktop.matrix_session.vault_corrupt', retryable: false }),
    new Error(JSON.stringify({ code: 'desktop.matrix_session.vault_corrupt', retryable: false })),
  ])('保留原生命令的稳定错误码 %#', async (error) => {
    const vault = new TauriMatrixSessionVault(vi.fn<MatrixVaultInvoke>().mockRejectedValue(error));
    await expect(vault.load()).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.matrix_session.vault_corrupt', retryable: false },
    });
  });

  it('明确报告命令权限配置缺失', async () => {
    const vault = new TauriMatrixSessionVault(
      vi
        .fn<MatrixVaultInvoke>()
        .mockRejectedValue('Command desktop_load_matrix_session not allowed by ACL'),
    );
    await expect(vault.load()).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.command.permission_denied' },
    });
  });

  it('不展示原生异常中的任意正文', async () => {
    const vault = new TauriMatrixSessionVault(
      vi.fn<MatrixVaultInvoke>().mockRejectedValue(new Error('unexpected secret')),
    );
    await expect(vault.load()).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.matrix_session.vault_unavailable' },
    });
  });
});
