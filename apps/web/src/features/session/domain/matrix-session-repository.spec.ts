import { describe, expect, it, vi } from 'vitest';

import { MatrixSessionRepository } from './matrix-session-repository';
import type { MatrixSessionVault, StoredMatrixSession } from './matrix-session-vault';
import type { SessionFailure } from './session';
import { err, ok } from '@/shared/result';

const session: StoredMatrixSession = {
  accessToken: 'old-access',
  deviceId: 'SAME_DEVICE',
  refreshToken: 'old-refresh',
  userId: '@tester:matrix.test',
  version: 1,
};
const rotated = { ...session, accessToken: 'new-access', refreshToken: 'new-refresh' };
const unavailable: SessionFailure = {
  boundary: 'matrix',
  code: 'desktop.matrix_session.vault_unavailable',
  offline: false,
  retryable: true,
};

function vault(initial: StoredMatrixSession | null = session) {
  let stored = initial;
  return {
    load: vi.fn<MatrixSessionVault['load']>(() => Promise.resolve(ok(stored))),
    save: vi.fn<MatrixSessionVault['save']>((value) => {
      stored = value;
      return Promise.resolve(ok(undefined));
    }),
    clear: vi.fn<MatrixSessionVault['clear']>(() => {
      stored = null;
      return Promise.resolve(ok(undefined));
    }),
  };
}

describe('MatrixSessionRepository', () => {
  it('重建仓储从同一个存储恢复用户与设备', async () => {
    const storage = vault(null);
    const first = new MatrixSessionRepository(storage);
    await first.save(session, first.epoch);
    await expect(new MatrixSessionRepository(storage).load()).resolves.toEqual(ok(session));
  });

  it('轮换令牌在落盘完成后才返回成功', async () => {
    const storage = vault();
    const repository = new MatrixSessionRepository(storage);
    await expect(
      repository.rotate(
        repository.epoch,
        () => true,
        () => Promise.resolve(rotated),
      ),
    ).resolves.toEqual(ok(rotated));
    await expect(storage.load()).resolves.toEqual(ok(rotated));
  });

  it('轮换落盘失败必须显式失败且下次恢复先重试最新令牌', async () => {
    const storage = vault();
    storage.save.mockResolvedValueOnce(err(unavailable));
    const repository = new MatrixSessionRepository(storage);
    await expect(
      repository.rotate(
        repository.epoch,
        () => true,
        () => Promise.resolve(rotated),
      ),
    ).resolves.toEqual(err(unavailable));
    expect(repository.failure).toEqual(unavailable);
    await expect(repository.load()).resolves.toEqual(ok(rotated));
    expect(storage.load).not.toHaveBeenCalled();
    expect(repository.failure).toBeNull();
  });

  it('退出阻止在途网络刷新把会话重新写回', async () => {
    const storage = vault();
    const repository = new MatrixSessionRepository(storage);
    const response = Promise.withResolvers<StoredMatrixSession>();
    const started = Promise.withResolvers<undefined>();
    const refreshing = repository.rotate(
      repository.epoch,
      () => true,
      () => {
        started.resolve(undefined);
        return response.promise;
      },
    );
    await started.promise;
    const clearing = repository.clear();
    response.resolve(rotated);
    await expect(refreshing).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.session_superseded' },
    });
    await expect(clearing).resolves.toEqual(ok(undefined));
    expect(storage.save).not.toHaveBeenCalled();
    await expect(storage.load()).resolves.toEqual(ok(null));
  });

  it('退出等待已开始的存储写入再执行清理', async () => {
    const storage = vault();
    const write = Promise.withResolvers<undefined>();
    const started = Promise.withResolvers<undefined>();
    const realSave = storage.save.getMockImplementation();
    storage.save.mockImplementationOnce(async (value) => {
      started.resolve(undefined);
      await write.promise;
      return realSave === undefined ? ok(undefined) : realSave(value);
    });
    const repository = new MatrixSessionRepository(storage);
    const saving = repository.save(rotated, repository.epoch);
    await started.promise;
    const clearing = repository.clear();
    write.resolve(undefined);
    await saving;
    await clearing;
    await expect(repository.load()).resolves.toEqual(ok(null));
  });

  it('重连先等待在途刷新完成而不是用旧令牌登录', async () => {
    const storage = vault();
    const repository = new MatrixSessionRepository(storage);
    const response = Promise.withResolvers<StoredMatrixSession>();
    const started = Promise.withResolvers<undefined>();
    const refreshing = repository.rotate(
      repository.epoch,
      () => true,
      () => {
        started.resolve(undefined);
        return response.promise;
      },
    );
    await started.promise;
    const restoring = repository.load();
    response.resolve(rotated);
    await refreshing;
    await expect(restoring).resolves.toEqual(ok(rotated));
  });

  it('旧连接和旧账户不能发起新的令牌轮换', async () => {
    const repository = new MatrixSessionRepository(vault());
    const oldEpoch = repository.epoch;
    const refresh = vi.fn(() => Promise.resolve(rotated));
    await repository.rotate(oldEpoch, () => false, refresh);
    repository.beginSession();
    await repository.rotate(oldEpoch, () => true, refresh);
    expect(refresh).not.toHaveBeenCalled();
  });

  it('读取和清理失败不能冒充未登录或退出成功', async () => {
    const storage = vault();
    storage.load.mockResolvedValue(err(unavailable));
    storage.clear.mockResolvedValue(err(unavailable));
    const repository = new MatrixSessionRepository(storage);
    await expect(repository.load()).resolves.toEqual(err(unavailable));
    await expect(repository.clear()).resolves.toEqual(err(unavailable));
  });
});
