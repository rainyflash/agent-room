// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { roomLiveViewQueryKey } from '@/features/private-rooms/data/room-live-view-queries';
import { roomExcerptsQueryKey } from '@/features/room-excerpts/data/room-excerpt-queries';
import type { RoomExcerptGateway } from '@/features/room-excerpts/domain/room-excerpts';
import { err, ok } from '@/shared/result';

import { useExcerptSharing } from './excerpt-selection';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const ROOM = '!room:example.org';
const LIVE_SINCE = new Date('2026-11-16T09:00:00Z').getTime();
const URL = 'https://agentroom.test/r/aB3xYz/s/Kq7Lm2Zp';

afterEach(cleanup);

/** 开实况以前一句（`m0`），以后 40 句（`m1`…`m40`）。 */
const TIMELINE: readonly RoomMessageSignal[] = Array.from({ length: 41 }, (_, index) =>
  conversationFixture(`m${String(index)}`, {
    serverTimestamp: LIVE_SINCE + (index - 1) * 60_000 + 1,
  }),
);

function message(index: number): RoomMessageSignal {
  const found = TIMELINE[index];
  if (found === undefined) throw new Error(`no message ${String(index)}`);
  return found;
}

function gateway(share: RoomExcerptGateway['share']): RoomExcerptGateway {
  return {
    share,
    list: vi.fn(() => Promise.resolve(ok([]))),
    remove: vi.fn(() => Promise.resolve(ok(undefined))),
  };
}

type Props = {
  readonly gateway: RoomExcerptGateway | undefined;
  readonly liveSinceUnixMs: number | null;
  readonly catalogId?: string;
  readonly timeline?: readonly RoomMessageSignal[];
};

function setup(initial: Props) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { readonly children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const hook = renderHook(
    (props: Props) =>
      useExcerptSharing({
        gateway: props.gateway,
        catalogId: props.catalogId ?? CATALOG,
        roomId: ROOM,
        liveSinceUnixMs: props.liveSinceUnixMs,
        timeline: props.timeline ?? TIMELINE,
      }),
    { initialProps: initial, wrapper },
  );
  return { ...hook, invalidate };
}

describe('选一段、分享', () => {
  it('没在放实况或没有接口时没有按钮', () => {
    const share = vi.fn<RoomExcerptGateway['share']>();
    expect(
      setup({ gateway: gateway(share), liveSinceUnixMs: null }).result.current.actions,
    ).toBeNull();
    cleanup();
    expect(
      setup({ gateway: undefined, liveSinceUnixMs: LIVE_SINCE }).result.current.actions,
    ).toBeNull();
  });

  it('只认开了实况以后真发出去的话', () => {
    const { result } = setup({ gateway: gateway(vi.fn()), liveSinceUnixMs: LIVE_SINCE });
    const actions = result.current.actions;
    expect(actions?.shareable(message(0))).toBe(false);
    expect(actions?.shareable(message(1))).toBe(true);
    expect(actions?.shareable({ ...message(1), matrixEventId: '~echo' })).toBe(false);
  });

  it('开头、往下点结尾、够不着时说为什么；Esc 取消', () => {
    const { result } = setup({ gateway: gateway(vi.fn()), liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(5)));
    expect(result.current.selected?.map((m) => m.messageId)).toEqual(['m5']);
    expect(result.current.actions?.place('m5')).toBe('only');

    act(() => result.current.actions?.pick(message(40)));
    expect(result.current.tooLong).toBe(true);
    expect(result.current.selected).toHaveLength(1);

    act(() => result.current.actions?.pick(message(34)));
    expect(result.current.tooLong).toBe(false);
    expect(result.current.selected).toHaveLength(30);
    expect(result.current.actions?.place('m5')).toBe('first');
    expect(result.current.actions?.place('m20')).toBe('middle');
    expect(result.current.actions?.place('m34')).toBe('last');
    expect(result.current.actions?.place('m35')).toBeUndefined();

    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(result.current.selected).toBeNull();
    expect(result.current.actions?.selecting).toBe(false);
  });

  it('点“分享”时才要链接，正文是开头和结尾那两句的事件 ID；成了以后重读分享过的，关掉就不再选', async () => {
    const share = vi.fn<RoomExcerptGateway['share']>(() =>
      Promise.resolve(ok({ code: 'Kq7Lm2Zp', url: URL })),
    );
    const { result, invalidate } = setup({ gateway: gateway(share), liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(3)));
    act(() => result.current.actions?.pick(message(8)));
    expect(share).not.toHaveBeenCalled();

    act(() => {
      result.current.share();
    });
    expect(result.current.dialog).toEqual({ kind: 'pending' });
    await waitFor(() => {
      expect(result.current.dialog).toEqual({ kind: 'shared', url: URL });
    });
    expect(share).toHaveBeenCalledOnce();
    expect(share).toHaveBeenCalledWith(CATALOG, {
      firstEventId: '$m3',
      lastEventId: '$m8',
    });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: roomExcerptsQueryKey(CATALOG) });

    act(() => {
      result.current.closeDialog();
    });
    expect(result.current.dialog).toBeNull();
    expect(result.current.selected).toBeNull();
  });

  it('没成时关掉还选着，好改了再分享；服务器说没在放实况时重问开关', async () => {
    const share = vi.fn<RoomExcerptGateway['share']>(() =>
      Promise.resolve(err({ code: 'room_excerpt.not_live', retryable: false })),
    );
    const { result, invalidate } = setup({ gateway: gateway(share), liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(3)));
    act(() => {
      result.current.share();
    });
    await waitFor(() => {
      expect(result.current.dialog).toEqual({
        kind: 'failed',
        failure: { code: 'room_excerpt.not_live', retryable: false },
      });
    });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: roomLiveViewQueryKey(CATALOG) });

    act(() => {
      result.current.closeDialog();
    });
    expect(result.current.dialog).toBeNull();
    expect(result.current.selected?.map((m) => m.messageId)).toEqual(['m3']);
  });

  it('连不上时说连不上，能再试一次', async () => {
    const share = vi
      .fn<RoomExcerptGateway['share']>()
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValueOnce(ok({ code: 'Kq7Lm2Zp', url: URL }));
    const { result } = setup({ gateway: gateway(share), liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(3)));
    act(() => {
      result.current.share();
    });
    await waitFor(() => {
      expect(result.current.dialog).toEqual({
        kind: 'failed',
        failure: { code: 'private_room.unreachable', retryable: true },
      });
    });
    act(() => {
      result.current.retry();
    });
    await waitFor(() => {
      expect(result.current.dialog).toEqual({ kind: 'shared', url: URL });
    });
  });

  it('实况关了、换了房间、开头那一句没了，都不再选', () => {
    const live = gateway(vi.fn());
    const { result, rerender } = setup({ gateway: live, liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(3)));
    rerender({ gateway: live, liveSinceUnixMs: null });
    expect(result.current.selected).toBeNull();

    rerender({ gateway: live, liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(3)));
    rerender({ gateway: live, liveSinceUnixMs: LIVE_SINCE, catalogId: 'another-room' });
    expect(result.current.selected).toBeNull();

    rerender({ gateway: live, liveSinceUnixMs: LIVE_SINCE });
    act(() => result.current.actions?.begin(message(3)));
    rerender({
      gateway: live,
      liveSinceUnixMs: LIVE_SINCE,
      timeline: TIMELINE.filter((m) => m.messageId !== 'm3'),
    });
    expect(result.current.selected).toBeNull();
  });
});
