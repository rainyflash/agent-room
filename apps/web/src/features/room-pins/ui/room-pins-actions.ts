import { createContext, useEffect, useRef, useState } from 'react';

import {
  ROOM_PINS_MAX,
  type RoomPin,
  type RoomPins,
  type RoomPinsGateway,
} from '@/features/room-pins/domain/room-pins';

/** 置顶或取消没成的几种，按人能做什么来分。 */
export type RoomPinsFailureKind =
  'full' | 'rate_limited' | 'forbidden' | 'archived' | 'not_a_message' | 'other';

/** 房主和管理员能做的：哪几句置顶着、哪几句正在改、置顶或取消一句（按 Matrix 事件 ID）。 */
export type RoomPinsActions = {
  /** Matrix 房间 ID：只给这间房的对话里的句子摆按钮。 */
  readonly roomId: string;
  readonly pinned: ReadonlySet<string>;
  readonly pending: ReadonlySet<string>;
  readonly toggle: (eventId: string, pin: boolean) => void;
};

/**
 * 对话里每一句的“置顶”“取消置顶”按钮从这里拿（房间页和句子之间隔着好几层）。没有它（不是管的人、
 * 私聊、围观页）就没有这个按钮。
 */
export const RoomPinsActionsContext = createContext<RoomPinsActions | null>(null);

export type RoomPinsControl = {
  /** 这一页上此刻的置顶：刚改完、房间状态还没同步过来时先照控制面回的那一份。 */
  readonly pins: RoomPins | undefined;
  /** 不是管的人时没有。 */
  readonly actions: RoomPinsActions | null;
  readonly failure: RoomPinsFailureKind | null;
  readonly dismissFailure: () => void;
};

type Confirmed = {
  /** 改之前房间状态里是哪一版：房间状态变了（同步过来了），就不再用控制面回的这一份。 */
  readonly basis: string;
  readonly pins: RoomPins | undefined;
};

/**
 * 房主和管理员置顶、取消置顶（`specs/redesign/brief-and-pins.md`“置顶”）：经控制面改，改完照它回的整份先
 * 显示，房间状态同步过来以后照房间状态。满了先在这里说，不去问控制面。换了房间从头来。
 */
export function useRoomPinsActions({
  gateway,
  catalogId,
  roomId,
  pins,
  manages,
  selfMatrixUserId,
}: {
  readonly gateway: RoomPinsGateway | undefined;
  readonly catalogId: string;
  readonly roomId: string;
  readonly pins: RoomPins | undefined;
  readonly manages: boolean;
  readonly selfMatrixUserId: string | null;
}): RoomPinsControl {
  const [scope, setScope] = useState(catalogId);
  const [pending, setPending] = useState<ReadonlySet<string>>(() => new Set());
  const [failure, setFailure] = useState<RoomPinsFailureKind | null>(null);
  const [confirmed, setConfirmed] = useState<Confirmed | null>(null);
  // 回来得晚的那一次，房间已经换了就不算。
  const current = useRef(catalogId);
  useEffect(() => {
    current.current = catalogId;
  }, [catalogId]);
  if (scope !== catalogId) {
    setScope(catalogId);
    setPending(new Set());
    setFailure(null);
    setConfirmed(null);
  }
  const basis = pinsVersion(pins);
  const effective = confirmed !== null && confirmed.basis === basis ? confirmed.pins : pins;
  const pinned = new Set(effective?.entries.map((pin) => pin.eventId) ?? []);

  const toggle = (eventId: string, pin: boolean): void => {
    if (gateway === undefined || pending.has(eventId)) return;
    if (pin && !pinned.has(eventId) && pinned.size >= ROOM_PINS_MAX) {
      setFailure('full');
      return;
    }
    setFailure(null);
    setPending((previous) => new Set(previous).add(eventId));
    const room = catalogId;
    void gateway.updatePins(room, eventId, pin).then((result) => {
      if (current.current !== room) return;
      setPending((previous) => {
        const next = new Set(previous);
        next.delete(eventId);
        return next;
      });
      if (!result.ok) {
        setFailure(failureKind(result.error.code));
        return;
      }
      setConfirmed({
        basis,
        pins: written(result.value, selfMatrixUserId),
      });
    });
  };

  return {
    pins: effective,
    actions: manages && gateway !== undefined ? { roomId, pinned, pending, toggle } : null,
    failure,
    dismissFailure: () => {
      setFailure(null);
    },
  };
}

function pinsVersion(pins: RoomPins | undefined): string {
  return pins === undefined
    ? ''
    : `${String(pins.updatedAtUnixMs)}:${pins.entries.map((pin) => pin.eventId).join(',')}`;
}

/** 控制面回的整份：最近一次是你改的，你不用再看一遍。 */
function written(entries: readonly RoomPin[], self: string | null): RoomPins | undefined {
  if (entries.length === 0) return undefined;
  return Object.freeze({
    entries: Object.freeze([...entries]),
    updatedAtUnixMs: Math.max(0, ...entries.map((pin) => pin.pinnedAtUnixMs)),
    updatedBy: self ?? '',
  });
}

export function failureKind(code: string): RoomPinsFailureKind {
  switch (code) {
    case 'room_pins.full':
      return 'full';
    case 'room_pins.rate_limited':
      return 'rate_limited';
    case 'room_pins.forbidden':
      return 'forbidden';
    case 'room_pins.conflict':
      return 'archived';
    case 'room_pins.not_a_message':
      return 'not_a_message';
    default:
      return 'other';
  }
}
