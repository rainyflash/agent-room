import { Link, useLocation } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { notificationAllowed, systemNotificationSummary } from '../domain/inbox';
import { useInbox } from './inbox-provider';
import './inbox.css';

function windowInFront(): boolean {
  return document.visibilityState === 'visible' && document.hasFocus();
}

export function InboxNotice() {
  const inbox = useInbox();
  const personal = usePersonalWorkspace();
  const desktop = useOptionalDesktopRuntimeController();
  const pathname = useLocation({ select: (location) => location.pathname });
  const { t } = useTranslation();
  const seen = useRef<{ account: string | null; ids: Set<string> } | null>(null);
  const [notice, setNotice] = useState<{ account: string; count: number } | null>(null);
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
    // The desktop keeps running in the tray: when the window is hidden or another app has focus,
    // the in-app banner is invisible, so the operating system carries the alert instead.
    if (desktop?.available === true && !windowInFront()) {
      const summary = systemNotificationSummary(fresh);
      void desktop.notify({
        title: summary.room ?? t('inbox.notification.title'),
        body:
          summary.count === 1 && summary.sender !== null
            ? t('inbox.notification.single', { sender: summary.sender, text: summary.text })
            : t('inbox.new', { count: summary.count }),
      });
    }
  }, [desktop, inbox, personal, t]);
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
  if (!notice || quiet || pathname === '/inbox' || notice.account !== inbox?.snapshot.accountId)
    return null;
  return (
    <aside className="inbox-notice" role="status">
      <Link to="/inbox">{t('inbox.new', { count: notice.count })}</Link>
      <button
        type="button"
        onClick={() => {
          setNotice(null);
        }}
      >
        {t('inbox.dismiss')}
      </button>
    </aside>
  );
}
