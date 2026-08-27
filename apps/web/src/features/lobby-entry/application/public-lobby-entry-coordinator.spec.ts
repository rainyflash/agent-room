import { describe, expect, it, vi } from 'vitest';

import { PublicLobbyEntryCoordinator } from '@/features/lobby-entry/application/public-lobby-entry-coordinator';
import { err, ok } from '@/shared/result';

const target = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  matrixRoomId: '!public-lobby:matrix.agent-room.test',
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
};

describe('公开大厅入场协调器', () => {
  it('先解析权威房间再确认 Matrix 加入成功', async () => {
    const resolve = vi.fn(() => Promise.resolve(ok(target)));
    const join = vi.fn(() => Promise.resolve(ok(undefined)));
    const coordinator = new PublicLobbyEntryCoordinator({ resolve }, { join });

    await expect(coordinator.enter(target.catalogId)).resolves.toEqual(ok(target));
    expect(resolve).toHaveBeenCalledWith(target.catalogId);
    expect(join).toHaveBeenCalledWith(target.matrixRoomId);
  });

  it('解析失败时绝不触碰 Matrix', async () => {
    const resolve = vi.fn(() =>
      Promise.resolve(err({ code: 'lobby_entry.not_found', retryable: false })),
    );
    const join = vi.fn(() => Promise.resolve(ok(undefined)));
    const coordinator = new PublicLobbyEntryCoordinator({ resolve }, { join });

    await expect(coordinator.enter(target.catalogId)).resolves.toEqual(
      err({ code: 'lobby_entry.not_found', retryable: false }),
    );
    expect(join).not.toHaveBeenCalled();
  });

  it('Matrix 加入失败时不把解析结果伪装成可进入目标', async () => {
    const coordinator = new PublicLobbyEntryCoordinator(
      { resolve: () => Promise.resolve(ok(target)) },
      {
        join: () =>
          Promise.resolve(err({ code: 'lobby_entry.matrix_join_failed', retryable: true })),
      },
    );

    await expect(coordinator.enter(target.catalogId)).resolves.toEqual(
      err({ code: 'lobby_entry.matrix_join_failed', retryable: true }),
    );
  });

  it('桌面已证明的房间跳过目录重选但仍要求 Web Matrix 成员关系', async () => {
    const resolve = vi.fn(() => Promise.resolve(ok(target)));
    const join = vi.fn(() => Promise.resolve(ok(undefined)));
    const coordinator = new PublicLobbyEntryCoordinator({ resolve }, { join });

    await expect(
      coordinator.enterKnown({ catalogId: target.catalogId, matrixRoomId: target.matrixRoomId }),
    ).resolves.toEqual(ok({ catalogId: target.catalogId, matrixRoomId: target.matrixRoomId }));
    expect(resolve).not.toHaveBeenCalled();
    expect(join).toHaveBeenCalledWith(target.matrixRoomId);
  });
});
