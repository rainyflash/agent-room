import { Link, useLocation } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { notificationAllowed } from '../domain/inbox';
import { useInbox } from './inbox-provider';
import './inbox.css';

export function InboxNotice() {
  const inbox = useInbox();
  const personal = usePersonalWorkspace();
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
    if (fresh.length > 0) setNotice({ account: accountId, count: fresh.length });
  }, [inbox, personal]);
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
