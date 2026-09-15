import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { ReadingPosition } from '@/features/personal-workspace/domain/workspace-document';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';

export function useConversationPosition({
  roomId,
  timeline,
  element,
  following,
  active,
  filtered,
  focusMessageId,
}: {
  readonly roomId: string;
  readonly timeline: readonly RoomMessageSignal[];
  readonly element: RefObject<HTMLDivElement | null>;
  readonly following: RefObject<boolean>;
  readonly active: boolean;
  readonly filtered: boolean;
  readonly focusMessageId: string | null;
}) {
  const personal = usePersonalWorkspace();
  const accountId = personal?.snapshot.accountId ?? null;
  const change = personal?.changeForAccount;
  const saved = personal?.snapshot.index.positions.get(roomId);
  const initialScope = useRef<string | null>(null);
  const lastFocus = useRef<string | null>(null);
  const pendingAnchor = useRef<ReadingPosition | null>(null);
  const [missing, setMissing] = useState(false);
  const ready = personal?.snapshot.status !== 'loading';
  useLayoutEffect(() => {
    if (!active || !ready || timeline.length === 0) return;
    const scope = JSON.stringify([accountId, roomId]);
    if (initialScope.current !== scope) {
      initialScope.current = scope;
      lastFocus.current = focusMessageId;
      pendingAnchor.current =
        focusMessageId !== null
          ? { messageId: focusMessageId, offset: 0, following: false }
          : (saved ?? null);
    } else if (focusMessageId !== lastFocus.current) {
      lastFocus.current = focusMessageId;
      if (focusMessageId !== null)
        pendingAnchor.current = { messageId: focusMessageId, offset: 0, following: false };
    }
    const anchor = pendingAnchor.current;
    const container = element.current;
    if (!anchor || !container) return;
    if (anchor.following) {
      following.current = true;
      pendingAnchor.current = null;
      setMissing(false);
      return;
    }
    following.current = false;
    const target = findMessage(container, anchor.messageId);
    if (target) {
      container.scrollTop +=
        target.getBoundingClientRect().top - container.getBoundingClientRect().top - anchor.offset;
      pendingAnchor.current = null;
      setMissing(false);
    } else setMissing(true);
  }, [accountId, active, element, focusMessageId, following, ready, roomId, saved, timeline]);

  const latest = timeline.at(-1);
  const markRead = useCallback(() => {
    if (
      accountId === null ||
      !change ||
      !latest ||
      !active ||
      filtered ||
      missing ||
      !following.current
    )
      return;
    change(accountId, {
      kind: 'read',
      id: roomId,
      value: { eventId: latest.matrixEventId, timestamp: latest.serverTimestamp },
    });
  }, [accountId, change, latest, active, filtered, missing, following, roomId]);

  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingSave = useRef<(() => void) | null>(null);
  const flush = useCallback(() => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
    pendingSave.current?.();
    pendingSave.current = null;
  }, []);
  useEffect(() => {
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      flush();
    };
  }, [flush]);

  const record = useCallback(() => {
    const container = element.current;
    if (!container || !active || filtered || missing || accountId === null || !change) return;
    const position = visibleAnchor(container, following.current);
    if (!position) return;
    pendingSave.current = () => {
      change(accountId, { kind: 'position', id: roomId, value: position });
    };
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = setTimeout(flush, 300);
  }, [element, active, filtered, missing, accountId, change, following, roomId, flush]);

  return {
    missing,
    markRead,
    record,
    preserve: () => {
      const container = element.current;
      if (container && !pendingAnchor.current)
        pendingAnchor.current = visibleAnchor(container, false);
      following.current = false;
    },
    latest: () => {
      pendingAnchor.current = null;
      following.current = true;
      setMissing(false);
    },
  };
}

function findMessage(container: HTMLElement, id: string): HTMLElement | undefined {
  return [...container.querySelectorAll<HTMLElement>('[data-conversation-message-id]')].find(
    (item) => item.dataset.conversationMessageId === id,
  );
}

function visibleAnchor(container: HTMLElement, following: boolean): ReadingPosition | null {
  const bounds = container.getBoundingClientRect();
  const nodes = [...container.querySelectorAll<HTMLElement>('[data-conversation-message-id]')];
  const target = following
    ? nodes.at(-1)
    : nodes.find((node) => node.getBoundingClientRect().bottom > bounds.top);
  const id = target?.dataset.conversationMessageId;
  return target && id
    ? {
        messageId: id,
        offset: Math.max(-10000, Math.min(10000, target.getBoundingClientRect().top - bounds.top)),
        following,
      }
    : null;
}
