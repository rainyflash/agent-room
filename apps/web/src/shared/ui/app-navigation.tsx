import { Link } from '@tanstack/react-router';
import { Compass, Footprints, Inbox, Settings } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { ThisComputerStatus } from '@/features/desktop/ui/this-computer-status';
import { useInbox } from '@/features/inbox/ui/inbox-provider';
import { SettingsAttentionDot } from '@/features/settings/ui/settings-attention';

type Destination = 'rooms' | 'agents' | 'inbox' | 'settings';
const destinations = [
  { id: 'rooms', to: '/rooms', icon: Compass },
  { id: 'inbox', to: '/inbox', icon: Inbox },
  { id: 'agents', to: '/workspace', search: {}, icon: Footprints },
  { id: 'settings', to: '/settings/$section', params: { section: 'general' }, icon: Settings },
] as const;

/**
 * 顶栏：房间 / 收件箱 / 我的 Agent / 设置。语言、版本和更新都在“设置”里；这台设备需要签名，
 * 或者桌面端查到新版本时，“设置”上有一个提醒点。
 */
export function AppNavigation({
  active,
  actions,
}: {
  readonly active?: Destination;
  readonly actions?: ReactNode;
}) {
  const { t } = useTranslation();
  const inbox = useInbox();
  const desktop = useOptionalDesktopRuntimeController();
  const unread = inbox?.snapshot.items.filter((item) => !item.read).length ?? 0;
  const updateReady = desktop?.available === true && desktop.update?.available === true;
  return (
    <header className="app-navigation">
      <div className="app-navigation__identity">
        <Link aria-label={t('app.name')} className="app-navigation__brand" to="/rooms">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
      </div>
      <nav aria-label={t('navigation.label')} className="app-navigation__links">
        {destinations.map(({ icon: Icon, id, ...destination }) => (
          <Link aria-current={active === id ? 'page' : undefined} {...destination} key={id}>
            <Icon aria-hidden="true" />
            <span>{t(`navigation.${id}`)}</span>
            {id === 'inbox' && unread > 0 ? <small>{unread > 99 ? '99+' : unread}</small> : null}
            {id === 'settings' ? <SettingsAttentionDot updateReady={updateReady} /> : null}
          </Link>
        ))}
      </nav>
      <div className="app-navigation__actions">
        {actions}
        <ThisComputerStatus />
      </div>
    </header>
  );
}
