import { ArrowUpRight, ChevronDown, Compass, Hash, Settings2, UserRound } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { DirectSessionNavigation } from '@/features/direct-sessions/ui/direct-session-navigation';
import type { DirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import { LanguageControl } from '@/features/preferences/ui/language-control';

export function WorkspaceNavigation({
  actions,
  activeDirectId,
  controller,
  onActivateRoom,
  onActivateDirect,
  roomName,
  userName,
}: {
  readonly actions: ReactNode;
  readonly activeDirectId: string | null;
  readonly controller: DirectSessionController;
  readonly onActivateRoom: () => void;
  readonly onActivateDirect: (catalogId: string) => void;
  readonly roomName: string;
  readonly userName: string | null;
}) {
  const { t } = useTranslation();
  return (
    <div className="workspace-navigation">
      <a href="/workspace" className="workspace-navigation__brand" aria-label={t('app.name')}>
        <img src="/agent-room-mark.svg" alt="" />
        <span>
          {t('app.name')}
          <small>{t('roomWorkspace.label')}</small>
        </span>
      </a>
      <nav className="workspace-navigation__rooms" aria-label={t('roomWorkspace.navigation')}>
        <p className="workspace-navigation__label">{t('roomWorkspace.spaces')}</p>
        <button
          className="workspace-navigation__room"
          type="button"
          aria-pressed={activeDirectId === null}
          onClick={onActivateRoom}
        >
          <Hash aria-hidden="true" />
          <span>
            <strong>{roomName}</strong>
            <small>{t('roomWorkspace.roomHint')}</small>
          </span>
        </button>
      </nav>
      <DirectSessionNavigation
        activeCatalogId={activeDirectId}
        controller={controller}
        onActivate={onActivateDirect}
      />
      <div className="workspace-navigation__footer">
        <a className="workspace-navigation__link" href="/rooms">
          <Compass aria-hidden="true" />
          {t('roomWorkspace.explore')}
          <ArrowUpRight aria-hidden="true" />
        </a>
        {actions === null ? null : (
          <details className="workspace-navigation__settings">
            <summary>
              <Settings2 aria-hidden="true" />
              {t('roomWorkspace.manage')}
              <ChevronDown aria-hidden="true" />
            </summary>
            <div className="workspace-navigation__actions">{actions}</div>
          </details>
        )}
        <LanguageControl />
        <a className="workspace-navigation__account" href="/workspace">
          <span>
            <UserRound aria-hidden="true" />
          </span>
          <div>
            <strong>{userName ?? t('roomWorkspace.guest')}</strong>
            <small>{t('roomWorkspace.account')}</small>
          </div>
          <ArrowUpRight aria-hidden="true" />
        </a>
      </div>
    </div>
  );
}
