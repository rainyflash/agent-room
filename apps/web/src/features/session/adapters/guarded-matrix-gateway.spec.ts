import { describe, expect, it, vi } from 'vitest';
import { createActor, waitFor } from 'xstate';
import { createSessionMachine } from '../domain/session-machine';
import type { MatrixGateway } from '../domain/session';
import { GuardedMatrixGateway } from './guarded-matrix-gateway';
import { err, ok } from '@/shared/result';

const user = '@human:matrix.test';
const homeserver = 'https://matrix.test';

function gateway() {
  return {
    beginAuthentication: vi
      .fn<MatrixGateway['beginAuthentication']>()
      .mockResolvedValue(ok({ kind: 'browser-navigation' })),
    disconnect: vi.fn(),
    logout: vi.fn<MatrixGateway['logout']>().mockResolvedValue(ok(undefined)),
    restore: vi
      .fn<MatrixGateway['restore']>()
      .mockResolvedValue(ok({ kind: 'authentication-required' })),
  };
}

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => {
      values.clear();
    },
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

describe('自动消息授权防循环', () => {
  it('状态机对无效的原生成功回调停止循环，手动重试成功后返回原房间', async () => {
    const matrix = gateway();
    matrix.beginAuthentication.mockResolvedValue(ok({ kind: 'session-established' }));
    const guarded = new GuardedMatrixGateway(matrix, memoryStorage(), homeserver);
    const navigate = vi.fn();
    const actor = createActor(
      createSessionMachine({
        browser: {
          currentPath: () => '/lobby/public',
          isOnline: () => true,
          replacePath: navigate,
        },
        matrix: guarded,
        privateState: { clear: vi.fn() },
        controlPlane: {
          beginAuthentication: () => Promise.resolve(ok({ kind: 'session-established' })),
          logout: () => Promise.resolve(ok(undefined)),
          readSession: () =>
            Promise.resolve(
              ok({
                authenticatedAtUnixMs: 1_800_000_000_000,
                expiresAtUnixMs: 1_900_000_000_000,
                displayName: 'Human',
                locale: 'en',
                matrixUserId: user,
                principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
                recentlyAuthenticated: true,
              }),
            ),
        },
      }),
    ).start();
    const failed = await waitFor(actor, (state) => state.matches('degraded'));
    expect(failed.context.failure?.code).toBe('matrix.authentication_interrupted');
    expect(matrix.beginAuthentication).toHaveBeenCalledOnce();
    expect(navigate).not.toHaveBeenCalled();
    matrix.beginAuthentication.mockImplementationOnce(() => {
      matrix.restore.mockResolvedValue(
        ok({
          kind: 'connected',
          connection: {
            deviceId: 'RECOVERED_DEVICE',
            userId: user,
            disconnect: vi.fn(),
            observe: () => () => undefined,
            waitUntilPrepared: () => Promise.resolve(ok(undefined)),
          },
        }),
      );
      return Promise.resolve(ok({ kind: 'session-established' }));
    });
    actor.send({ type: 'RETRY' });
    await waitFor(actor, (state) => state.matches('ready'));
    expect(matrix.beginAuthentication).toHaveBeenCalledTimes(2);
    expect(navigate).toHaveBeenCalledExactlyOnceWith('/lobby/public');
    actor.stop();
  });

  it('首次自动授权后，返回或重载页面不会再次自动跳转，显式重试只发起一次新授权', async () => {
    const storage = memoryStorage();
    const matrix = gateway();
    const first = new GuardedMatrixGateway(matrix, storage, homeserver);
    await first.restore(user);
    expect((await first.beginAuthentication('/lobby/public', 'automatic')).ok).toBe(true);

    const returned = new GuardedMatrixGateway(matrix, storage, homeserver);
    await returned.restore(user);
    await expect(returned.beginAuthentication('/connect', 'automatic')).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.authentication_interrupted', retryable: true },
    });
    expect(matrix.beginAuthentication).toHaveBeenCalledExactlyOnceWith('/lobby/public');
    expect((await returned.beginAuthentication('/connect', 'interactive')).ok).toBe(true);
    expect(matrix.beginAuthentication).toHaveBeenCalledTimes(2);
  });

  it('恢复已保存的通信身份会解除授权标记，且不会创建新设备或清除凭据', async () => {
    const storage = memoryStorage();
    const matrix = gateway();
    const guarded = new GuardedMatrixGateway(matrix, storage, homeserver);
    await guarded.restore(user);
    await guarded.beginAuthentication('/rooms', 'automatic');
    matrix.restore.mockResolvedValueOnce(
      ok({
        kind: 'connected',
        connection: {
          deviceId: 'ORIGINAL_DEVICE',
          userId: user,
          disconnect: vi.fn(),
          observe: () => () => undefined,
          waitUntilPrepared: () => Promise.resolve(ok(undefined)),
        },
      }),
    );
    const restored = await guarded.restore(user);
    expect(restored).toMatchObject({
      ok: true,
      value: { connection: { deviceId: 'ORIGINAL_DEVICE' } },
    });
    expect(storage.length).toBe(0);
    expect(matrix.logout).not.toHaveBeenCalled();
    expect(matrix.beginAuthentication).toHaveBeenCalledOnce();
  });

  it('桌面授权返回成功但没有建立会话时，同样停止自动重开浏览器', async () => {
    const matrix = gateway();
    matrix.beginAuthentication.mockResolvedValue(ok({ kind: 'session-established' }));
    const guarded = new GuardedMatrixGateway(matrix, memoryStorage(), homeserver);
    await guarded.restore(user);
    await guarded.beginAuthentication('/rooms', 'automatic');
    await guarded.restore(user);
    await expect(guarded.beginAuthentication('/rooms', 'automatic')).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.authentication_interrupted' },
    });
    expect(matrix.beginAuthentication).toHaveBeenCalledOnce();
  });

  it('不同账户和服务器的自动授权互不阻塞，主动退出会清理当前记录', async () => {
    const storage = memoryStorage();
    const matrix = gateway();
    const first = new GuardedMatrixGateway(matrix, storage, homeserver);
    await first.restore(user);
    await first.beginAuthentication('/rooms', 'automatic');
    const otherServer = new GuardedMatrixGateway(matrix, storage, 'https://other.test');
    await otherServer.restore(user);
    expect((await otherServer.beginAuthentication('/rooms', 'automatic')).ok).toBe(true);
    await first.restore('@other:matrix.test');
    expect((await first.beginAuthentication('/rooms', 'automatic')).ok).toBe(true);
    expect((await first.logout()).ok).toBe(true);
    expect(storage.length).toBe(1);
  });

  it('无法记录授权时明确失败，不启动无法防止循环的跳转', async () => {
    const matrix = gateway();
    const storage = memoryStorage();
    storage.setItem = () => {
      throw new Error('Storage blocked');
    };
    const guarded = new GuardedMatrixGateway(matrix, storage, homeserver);
    await guarded.restore(user);
    await expect(guarded.beginAuthentication('/rooms', 'automatic')).resolves.toMatchObject({
      ok: false,
      error: { code: 'browser.session_storage_unavailable' },
    });
    expect(matrix.beginAuthentication).not.toHaveBeenCalled();
  });

  it('授权失败保留真实错误，只有用户重试才可再次发起授权', async () => {
    const matrix = gateway();
    matrix.beginAuthentication.mockResolvedValue(
      err({
        boundary: 'matrix',
        code: 'desktop.matrix_session.loopback_timeout',
        offline: false,
        retryable: true,
      }),
    );
    const guarded = new GuardedMatrixGateway(matrix, memoryStorage(), homeserver);
    await guarded.restore(user);
    await expect(guarded.beginAuthentication('/rooms', 'automatic')).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.matrix_session.loopback_timeout' },
    });
    await guarded.beginAuthentication('/rooms', 'automatic');
    expect(matrix.beginAuthentication).toHaveBeenCalledOnce();
  });
});
