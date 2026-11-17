import { useMutation, useQueryClient } from '@tanstack/react-query';
import { createContext, useEffect, useMemo, useRef, useState } from 'react';

import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { roomLiveViewQueryKey } from '@/features/private-rooms/data/room-live-view-queries';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import { roomExcerptsQueryKey } from '@/features/room-excerpts/data/room-excerpt-queries';
import {
  excerptMessages,
  excerptPlaces,
  excerptShareable,
  pickExcerptMessage,
  type ExcerptPlace,
  type ExcerptRange,
  type RoomExcerptGateway,
} from '@/features/room-excerpts/domain/room-excerpts';

/**
 * 对话里每一句的“分享这一段”和选的时候那个勾从这里拿（房间页和句子之间隔着好几层）。没有它（房间没在放
 * 实况、私聊、围观页、子房间）就没有这个按钮。
 */
export type ExcerptSelectionActions = {
  /** Matrix 房间 ID：只给这间房的对话里的句子摆按钮。 */
  readonly roomId: string;
  /** 正在选。 */
  readonly selecting: boolean;
  readonly shareable: (message: RoomMessageSignal) => boolean;
  /** 这一句在选中的那一段里排在哪；不在里面是 `undefined`。 */
  readonly place: (messageId: string) => ExcerptPlace | undefined;
  /** 从这一句开始选。 */
  readonly begin: (message: RoomMessageSignal) => void;
  /** 选的时候点了这一句。 */
  readonly pick: (message: RoomMessageSignal) => void;
};

export const ExcerptSelectionContext = createContext<ExcerptSelectionActions | null>(null);

/** 分享的对话框此刻是什么样：在生成链接、生成好了、没成。 */
export type ExcerptShareState =
  | { readonly kind: 'pending' }
  | { readonly kind: 'shared'; readonly url: string }
  | { readonly kind: 'failed'; readonly failure: PrivateRoomFailure };

/** 让实况那一份重问的几种：服务器说没在放了、房间不在了，按钮多半该收起来了。 */
const REFRESH_LIVE_VIEW = new Set([
  'room_excerpt.not_live',
  'room_excerpt.not_found',
  'room_excerpt.forbidden',
]);

export type ExcerptSharing = {
  /** 给对话里每一句的；不能分享时是 `null`。 */
  readonly actions: ExcerptSelectionActions | null;
  /** 正在选时选中的那几句（至少一句）；没在选是 `null`。 */
  readonly selected: readonly RoomMessageSignal[] | null;
  /** 刚才点的那一句够不着（超过 30 句）。 */
  readonly tooLong: boolean;
  /** 分享的对话框开着时它的样子；关着是 `null`。 */
  readonly dialog: ExcerptShareState | null;
  readonly share: () => void;
  readonly retry: () => void;
  readonly cancel: () => void;
  readonly closeDialog: () => void;
};

/**
 * 对话里“分享这一段”（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7b）：点了哪一句，那一句是开头，
 * 往下点到哪一句就是结尾，最多 30 句、连着的；对话上面那一条点“分享”就向控制面要链接，对话框里给出来。
 * 只在房间放着实况时有（`liveSinceUnixMs` 不是 `null`），也只认开了实况以后的话。换了房间、实况关了、开头
 * 那一句没了，都不再选。
 */
export function useExcerptSharing({
  gateway,
  catalogId,
  roomId,
  liveSinceUnixMs,
  timeline,
}: {
  readonly gateway: RoomExcerptGateway | undefined;
  readonly catalogId: string;
  readonly roomId: string;
  /** 实况打开的那一刻；没在放是 `null`。 */
  readonly liveSinceUnixMs: number | null;
  /** 这间房的对话，按先后（`conversationMessages`）。 */
  readonly timeline: readonly RoomMessageSignal[];
}): ExcerptSharing {
  const queryClient = useQueryClient();
  const enabled = gateway !== undefined && liveSinceUnixMs !== null;
  const [scope, setScope] = useState(catalogId);
  const [range, setRange] = useState<ExcerptRange | null>(null);
  const [tooLong, setTooLong] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  // 开始选、取消以后，焦点放回哪一句的哪个按钮（那一句的按钮换了一个）。
  const [focus, setFocus] = useState<{
    readonly messageId: string;
    readonly target: 'select' | 'share' | 'message';
  } | null>(null);
  // 记住：每一句都从上下文里读它，换一份就整段对话重画。
  const selected = useMemo(
    () => (range === null ? null : excerptMessages(timeline, range)),
    [range, timeline],
  );
  if (scope !== catalogId || (range !== null && (!enabled || selected === null))) {
    setScope(catalogId);
    setRange(null);
    setTooLong(false);
    setDialogOpen(false);
  }
  const mutation = useMutation({
    mutationFn: async (input: {
      readonly catalogId: string;
      readonly firstEventId: string;
      readonly lastEventId: string;
    }) => {
      if (gateway === undefined) throw new Error('room excerpts are unavailable');
      return await gateway.share(input.catalogId, {
        firstEventId: input.firstEventId,
        lastEventId: input.lastEventId,
      });
    },
    onSettled: (result, _error, input) => {
      if (result?.ok === true) {
        void queryClient.invalidateQueries({ queryKey: roomExcerptsQueryKey(input.catalogId) });
      } else if (result !== undefined && REFRESH_LIVE_VIEW.has(result.error.code)) {
        void queryClient.invalidateQueries({ queryKey: roomLiveViewQueryKey(input.catalogId) });
      }
    },
  });
  const places = useMemo(() => excerptPlaces(selected ?? []), [selected]);

  useEffect(() => {
    if (focus === null) return;
    setFocus(null);
    const wrapper = document.querySelector<HTMLElement>(
      `[data-conversation-message-id="${CSS.escape(focus.messageId)}"]`,
    );
    const target =
      focus.target === 'message'
        ? wrapper
        : wrapper?.querySelector<HTMLElement>(
            focus.target === 'select'
              ? '.conversation-message__select'
              : '.conversation-message__share',
          );
    target?.focus({ preventScroll: focus.target !== 'message' });
  }, [focus]);

  const selecting = enabled && selected !== null;
  const cancel = (): void => {
    if (range !== null) setFocus({ messageId: range.startId, target: 'share' });
    setRange(null);
    setTooLong(false);
  };
  // Esc 取消；对话框开着时 Esc 只关对话框。
  const escape = useRef(cancel);
  useEffect(() => {
    escape.current = cancel;
  });
  useEffect(() => {
    if (!selecting || dialogOpen) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      if (event.target instanceof Element && event.target.closest('dialog') !== null) return;
      escape.current();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [selecting, dialogOpen]);

  const actions = useMemo((): ExcerptSelectionActions | null => {
    if (gateway === undefined || liveSinceUnixMs === null) return null;
    const since = liveSinceUnixMs;
    return {
      roomId,
      selecting,
      shareable: (message) => excerptShareable(message, since),
      place: (messageId) => places.get(messageId),
      begin: (message) => {
        setRange({ startId: message.messageId, endId: message.messageId });
        setTooLong(false);
        setFocus({ messageId: message.messageId, target: 'select' });
      },
      pick: (message) => {
        if (range === null) return;
        const picked = pickExcerptMessage(timeline, range, message.messageId);
        setRange(picked.range);
        setTooLong(picked.tooLong);
      },
    };
  }, [gateway, roomId, selecting, liveSinceUnixMs, places, range, timeline]);

  const request = (): void => {
    const first = selected?.[0];
    const last = selected?.at(-1);
    if (first === undefined || last === undefined) return;
    mutation.mutate({
      catalogId,
      firstEventId: first.matrixEventId,
      lastEventId: last.matrixEventId,
    });
  };

  const dialog: ExcerptShareState | null = !dialogOpen
    ? null
    : mutation.isPending || mutation.isIdle
      ? { kind: 'pending' }
      : mutation.data?.ok === true
        ? { kind: 'shared', url: mutation.data.value.url }
        : {
            kind: 'failed',
            failure:
              mutation.data?.ok === false
                ? mutation.data.error
                : { code: 'private_room.unreachable', retryable: true },
          };

  return {
    actions,
    selected: selecting ? selected : null,
    tooLong,
    dialog,
    share: () => {
      if (!selecting || mutation.isPending) return;
      setDialogOpen(true);
      request();
    },
    retry: request,
    cancel,
    closeDialog: () => {
      const shared = mutation.data?.ok === true;
      setDialogOpen(false);
      mutation.reset();
      if (shared && range !== null) {
        // 分享好了：这一段做完了，不再选；焦点回到对话里那一段的开头。
        setFocus({ messageId: range.startId, target: 'message' });
        setRange(null);
        setTooLong(false);
      }
    },
  };
}
