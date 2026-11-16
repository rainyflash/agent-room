import { Button, Toast } from '@agent-room/ui-system';
import { Link, useLocation, useNavigate } from '@tanstack/react-router';
import { BellRing } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { windowInForeground } from '@/shared/browser/window-foreground';
import { notificationAllowed, systemNotificationSummary, type InboxItem } from '../domain/inbox';
import { useInbox } from './inbox-provider';
import { useWebNotifications } from './web-notifications-provider';
import './inbox.css';

export function InboxNotice() {
  const inbox = useInbox();
  const personal = usePersonalWorkspace();
  const desktop = useOptionalDesktopRuntimeController();
  const notifications = useWebNotifications();
  const navigate = useNavigate();
  const pathname = useLocation({ select: (location) => location.pathname });
  const { t } = useTranslation();
  const seen = useRef<{ account: string | null; ids: Set<string> } | null>(null);
  const [notice, setNotice] = useState<{ account: string; count: number } | null>(null);
  const [offer, setOffer] = useState<string | null>(null);
  useEffect(() => {
    if (!inbox || !personal || inbox.snapshot.status !== 'ready') return;
    const { accountId, items } = inbox.snapshot;
    const previous = seen.current;
    seen.current = { account: accountId, ids: new Set(items.map((item) => item.id)) };
    // Starting the app, loading older history and changing accounts must not replay alerts.
    if (!accountId || previous?.account !== accountId) return;
    const fresh = items.filter(
      (item) =>
        !previous.ids.has(item.id) &&
        item.timestamp > Date.now() - 120000 &&
        notificationAllowed(item, personal.snapshot.index, Date.now()),
    );
    if (fresh.length === 0) return;
    setNotice({ account: accountId, count: fresh.length });
    // The in-app banner is invisible while the window is hidden or another app has focus,
    // so the operating system carries the alert instead: the desktop's own, or the browser's.
    const summary = systemNotificationSummary(fresh);
    const title = summary.room ?? t('inbox.notification.title');
    const body =
      summary.count === 1 && summary.sender !== null
        ? t('inbox.notification.single', { sender: summary.sender, text: summary.text })
        : t('inbox.new', { count: summary.count });
    if (desktop?.available === true) {
      if (!windowInForeground()) void desktop.notify({ title, body });
      return;
    }
    if (!windowInForeground()) {
      const only = fresh.length === 1 ? fresh[0] : undefined;
      notifications.show({
        body,
        onClick: () => {
          window.focus();
          void navigate(only === undefined ? { to: '/inbox' } : itemTarget(only));
        },
        tag: summary.room === null ? 'agent-room-inbox' : `agent-room-room:${summary.room}`,
        title,
      });
    }
    // 第一次有人找你、你又没开过网页通知：问一句，只问一次（4R-2）。
    const state = notifications.state();
    if (!state.offered && !state.wanted && state.permission === 'default') setOffer(accountId);
  }, [desktop, inbox, navigate, notifications, personal, t]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => {
      setNotice(null);
    }, 8000);
    return () => {
      clearTimeout(timer);
    };
  }, [notice]);
  const quiet =
    personal?.snapshot.index.doNotDisturbUntil === null ||
    (personal?.snapshot.index.doNotDisturbUntil ?? 0) > Date.now();
  if (quiet) return null;
  const account = inbox?.snapshot.accountId;
  const answered = () => {
    notifications.markOffered();
    setOffer(null);
  };
  return (
    <>
      {notice && pathname !== '/inbox' && notice.account === account ? (
        <Toast
          action={
            <Link className="ar-button ar-button--compact ar-button--primary" to="/inbox">
              {t('inbox.openInbox')}
            </Link>
          }
          dismissLabel={t('inbox.dismiss')}
          onDismiss={() => {
            setNotice(null);
          }}
          title={t('inbox.new', { count: notice.count })}
        />
      ) : null}
      {offer !== null && offer === account ? (
        <Toast
          action={
            <>
              <Button onClick={answered} size="compact" tone="ghost">
                {t('inbox.webNotifications.offerLater')}
              </Button>
              <Button
                onClick={() => {
                  void notifications.setWanted(true).finally(answered);
                }}
                size="compact"
                tone="primary"
              >
                {t('inbox.webNotifications.offerAccept')}
              </Button>
            </>
          }
          dismissLabel={t('inbox.dismiss')}
          icon={<BellRing aria-hidden="true" />}
          onDismiss={answered}
          title={t('inbox.webNotifications.offerTitle')}
        >
          {t('inbox.webNotifications.offerBody')}
        </Toast>
      ) : null}
    </>
  );
}

function itemTarget(item: InboxItem) {
  return {
    params: { catalogId: item.room.catalogId, roomId: item.room.roomId },
    search: {
      message: item.messageId,
      view: item.conversation ? ('conversation' as const) : ('resources' as const),
      ...(item.room.direct ? { direct: item.room.catalogId } : {}),
    },
    to: '/lobby/$catalogId/instance/$roomId' as const,
  };
}
