// @vitest-environment jsdom

import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { RoomPin, RoomPins, RoomPinsGateway } from '@/features/room-pins/domain/room-pins';
import { failureKind, useRoomPinsActions } from '@/features/room-pins/ui/room-pins-actions';
import { err, ok, type Result } from '@/shared/result';

const SELF = '@mina:matrix.test';
const GRACE = '@grace:matrix.test';
const AT = Date.parse('2026-11-16T08:00:00.000Z');

const pin = (eventId: string, by = GRACE, at = AT): RoomPin => ({
  eventId,
  pinnedBy: by,
  pinnedAtUnixMs: at,
});
const roomPins = (ids: readonly string[], at = AT): RoomPins => ({
  entries: ids.map((id) => pin(id)),
  updatedAtUnixMs: at,
  updatedBy: GRACE,
});

type Reply = Result<readonly RoomPin[], PrivateRoomFailure>;

/** 控制面：每次调用先挂着，测试里再一个个答。 */
function gateway() {
  const replies: ((reply: Reply) => void)[] = [];
  const updatePins = vi.fn<RoomPinsGateway['updatePins']>(
    () =>
      new Promise<Reply>((resolve) => {
        replies.push(resolve);
      }),
  );
  return {
    gateway: { updatePins } satisfies RoomPinsGateway,
    updatePins,
    answer: async (reply: Reply) => {
      await act(async () => {
        replies.shift()?.(reply);
        await Promise.resolve();
      });
    },
  };
}

function setup(pins: RoomPins | undefined, catalogId = 'room-a', manages = true) {
  const control = gateway();
  const view = renderHook(
    (props: { readonly pins: RoomPins | undefined; readonly catalogId: string }) =>
      useRoomPinsActions({
        gateway: control.gateway,
        catalogId: props.catalogId,
        roomId: '!club:matrix.test',
        pins: props.pins,
        manages,
        selfMatrixUserId: SELF,
      }),
    { initialProps: { pins, catalogId } },
  );
  return { ...control, view };
}

describe('房主和管理员置顶、取消置顶', () => {
  it('不是管的人没有按钮可用', () => {
    const { view } = setup(roomPins(['$a']), 'room-a', false);
    expect(view.result.current.actions).toBeNull();
    expect(view.result.current.pins?.entries.map((entry) => entry.eventId)).toEqual(['$a']);
  });

  it('改的时候那一句先关着；改完照控制面回的先显示，是你改的不亮点；房间状态同步过来以后照房间状态', async () => {
    const { view, updatePins, answer } = setup(roomPins(['$a']));
    act(() => {
      view.result.current.actions?.toggle('$b', true);
    });
    expect(updatePins).toHaveBeenCalledWith('room-a', '$b', true);
    expect(view.result.current.actions?.pending.has('$b')).toBe(true);
    // 正在改的同一句再点不再发。
    act(() => {
      view.result.current.actions?.toggle('$b', true);
    });
    expect(updatePins).toHaveBeenCalledTimes(1);

    await answer(ok([pin('$b', SELF, AT + 1), pin('$a')]));

    expect(view.result.current.actions?.pending.size).toBe(0);
    expect(view.result.current.pins?.entries.map((entry) => entry.eventId)).toEqual(['$b', '$a']);
    expect(view.result.current.pins?.updatedBy).toBe(SELF);
    expect(view.result.current.actions?.pinned.has('$b')).toBe(true);

    // 同步过来的房间状态说了算（这里别人又取消了一条）。
    view.rerender({ pins: roomPins(['$b'], AT + 2), catalogId: 'room-a' });
    expect(view.result.current.pins?.entries.map((entry) => entry.eventId)).toEqual(['$b']);
  });

  it('取消了最后一条就没有置顶了', async () => {
    const { view, answer } = setup(roomPins(['$a']));
    act(() => {
      view.result.current.actions?.toggle('$a', false);
    });
    await answer(ok([]));
    expect(view.result.current.pins).toBeUndefined();
  });

  it('满了先在这里说，不去问控制面；控制面说不行的照错误码说，关掉或者再改一次就收起来', async () => {
    const { view, updatePins, answer } = setup(roomPins(['$1', '$2', '$3', '$4', '$5']));
    act(() => {
      view.result.current.actions?.toggle('$6', true);
    });
    expect(updatePins).not.toHaveBeenCalled();
    expect(view.result.current.failure).toBe('full');

    act(() => {
      view.result.current.actions?.toggle('$1', false);
    });
    expect(view.result.current.failure).toBeNull();
    await answer(err({ code: 'room_pins.rate_limited', retryable: true }));
    expect(view.result.current.failure).toBe('rate_limited');
    expect(view.result.current.actions?.pending.size).toBe(0);

    act(() => {
      view.result.current.dismissFailure();
    });
    expect(view.result.current.failure).toBeNull();
  });

  it('换了房间从头来：上一间回来得晚的不算', async () => {
    const { view, answer } = setup(roomPins(['$a']));
    act(() => {
      view.result.current.actions?.toggle('$b', true);
    });
    view.rerender({ pins: undefined, catalogId: 'room-b' });
    await answer(err({ code: 'room_pins.forbidden', retryable: false }));
    expect(view.result.current.failure).toBeNull();
    expect(view.result.current.actions?.pending.size).toBe(0);
  });

  it('错误码换成人能做什么', () => {
    expect(failureKind('room_pins.full')).toBe('full');
    expect(failureKind('room_pins.forbidden')).toBe('forbidden');
    expect(failureKind('room_pins.conflict')).toBe('archived');
    expect(failureKind('room_pins.not_a_message')).toBe('not_a_message');
    expect(failureKind('room_pins.dependency_unavailable')).toBe('other');
  });
});
