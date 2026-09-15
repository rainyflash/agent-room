import { Link } from '@tanstack/react-router';
import { Compass, Footprints, Inbox, ShieldCheck } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';
import { useInbox } from '@/features/inbox/ui/inbox-provider';
import { ApplicationVersionLink } from '@/features/updates/ui/application-version-link';

type Destination = 'rooms' | 'agents' | 'inbox' | 'security';
const destinations = [
  { id: 'rooms', to: '/rooms', icon: Compass },
  { id: 'inbox', to: '/inbox', icon: Inbox },
  { id: 'agents', to: '/workspace', search: {}, icon: Footprints },
  { id: 'security', to: '/settings/$section', params: { section: 'security' }, icon: ShieldCheck },
] as const;

export function AppNavigation({
  active,
  actions,
}: {
  readonly active?: Destination;
  readonly actions?: ReactNode;
}) {
  const { t } = useTranslation();
  const inbox = useInbox();
  const unread = inbox?.snapshot.items.filter((item) => !item.read).length ?? 0;
  return (
    <header className="app-navigation">
      <div className="app-navigation__identity">
        <Link aria-label={t('app.name')} className="app-navigation__brand" to="/rooms">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
        <ApplicationVersionLink />
      </div>
      <nav aria-label={t('navigation.label')} className="app-navigation__links">
        {destinations.map(({ icon: Icon, id, ...destination }) => (
          <Link aria-current={active === id ? 'page' : undefined} {...destination} key={id}>
            <Icon aria-hidden="true" />
            <span>{t(`navigation.${id}`)}</span>
            {id === 'inbox' && unread > 0 ? <small>{unread > 99 ? '99+' : unread}</small> : null}
          </Link>
        ))}
      </nav>
      <div className="app-navigation__actions">
        {actions}
        <LanguageControl />
      </div>
    </header>
  );
}
