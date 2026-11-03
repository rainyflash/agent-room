// @vitest-environment jsdom

import { renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ExploreRoomDetail } from '@/features/explore/domain/explore';
import type { SessionStateName } from '@/features/session/ui/connection-model';
import { err, ok } from '@/shared/result';

const navigate = vi.fn();
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => navigate }));

const { useJoinContinuation } = await import('./use-join-continuation');

const room: ExploreRoomDetail = {
  admins: [],
  agentCount: 0,
  catalogId: '0198c000-0000-7000-8000-000000000001',
  createdAtUnixMs: 0,
  description: '',
  endToEndEncrypted: false,
  joinMode: 'open',
  lastMessageAtUnixMs: null,
  memberCount: 1,
  name: 'Rust help desk',
  online: null,
  ownerName: 'Ada',
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
  viewer: null,
};

const handlers = { onHandled: vi.fn(), onJoin: vi.fn(), onSignIn: vi.fn() };

type Props = Parameters<typeof useJoinContinuation>[0];

function props(overrides: Partial<Props>): Props {
  return {
    continuation: { signup: false },
    detail: ok(room),
    registrationOpen: true,
    selectedCode: room.shareCode,
    sessionState: 'ready' satisfies SessionStateName,
    signedIn: true,
    ...handlers,
    ...overrides,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe('从分享页点“加入”、登录回来接着做', () => {
  it('没登录：先从地址上去掉 join，再去登录；点了注册的去注册', () => {
    renderHook(() =>
      useJoinContinuation(
        props({ continuation: { signup: true }, sessionState: 'unauthenticated', signedIn: false }),
      ),
    );
    expect(handlers.onHandled).toHaveBeenCalledOnce();
    expect(handlers.onSignIn).toHaveBeenCalledWith('aB3xYz', 'register');
    expect(handlers.onHandled.mock.invocationCallOrder[0]).toBeLessThan(
      handlers.onSignIn.mock.invocationCallOrder[0] ?? 0,
    );
  });

  it('注册没开放时点了注册也去登录', () => {
    renderHook(() =>
      useJoinContinuation(
        props({
          continuation: { signup: true },
          registrationOpen: false,
          sessionState: 'unauthenticated',
          signedIn: false,
        }),
      ),
    );
    expect(handlers.onSignIn).toHaveBeenCalledWith('aB3xYz', 'sign-in');
  });

  it('消息还没连好、详情还没读到时先等着，地址上的 join 留着', () => {
    const { rerender } = renderHook((current: Props) => useJoinContinuation(current), {
      initialProps: props({ sessionState: 'syncing' }),
    });
    rerender(props({ detail: undefined }));
    expect(handlers.onHandled).not.toHaveBeenCalled();
    expect(handlers.onJoin).not.toHaveBeenCalled();

    rerender(props({}));
    expect(handlers.onJoin).toHaveBeenCalledWith(room);
    expect(handlers.onHandled).toHaveBeenCalledOnce();
    // 只做一次：再渲染也不再加入。
    rerender(props({}));
    expect(handlers.onJoin).toHaveBeenCalledOnce();
  });

  it('已经在里面的直接进房间', () => {
    renderHook(() => useJoinContinuation(props({ detail: ok({ ...room, viewer: 'joined' }) })));
    expect(navigate).toHaveBeenCalledWith({
      params: { catalogId: room.catalogId },
      replace: true,
      search: {},
      to: '/lobby/$catalogId',
    });
    expect(handlers.onJoin).not.toHaveBeenCalled();
  });

  it('要申请的把光标放进申请框，不替他申请', () => {
    const { result } = renderHook(() =>
      useJoinContinuation(props({ detail: ok({ ...room, joinMode: 'request' }) })),
    );
    expect(result.current).toBe(true);
    expect(handlers.onJoin).not.toHaveBeenCalled();
    expect(handlers.onHandled).toHaveBeenCalledOnce();
  });

  it('房间不公开了：去掉 join，详情上自己会说', () => {
    renderHook(() =>
      useJoinContinuation(
        props({ detail: err({ code: 'public_room.not_found', retryable: false }) }),
      ),
    );
    expect(handlers.onHandled).toHaveBeenCalledOnce();
    expect(handlers.onJoin).not.toHaveBeenCalled();
  });

  it('不是从分享页来的什么都不做', () => {
    renderHook(() => useJoinContinuation(props({ continuation: null })));
    expect(handlers.onHandled).not.toHaveBeenCalled();
    expect(handlers.onJoin).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });
});
