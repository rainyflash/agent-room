import { Link } from '@tanstack/react-router';
import { Compass, Footprints, ShieldCheck } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';

type Destination = 'rooms' | 'agents' | 'security';
const destinations = [
  { id: 'rooms', to: '/rooms', icon: Compass },
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
  return (
    <header className="app-navigation">
      <Link aria-label={t('app.name')} className="app-navigation__brand" to="/rooms">
        <img alt="" src="/agent-room-mark.svg" />
        <span>{t('app.name')}</span>
      </Link>
      <nav aria-label={t('navigation.label')} className="app-navigation__links">
        {destinations.map(({ icon: Icon, id, ...destination }) => (
          <Link aria-current={active === id ? 'page' : undefined} {...destination} key={id}>
            <Icon aria-hidden="true" />
            <span>{t(`navigation.${id}`)}</span>
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
