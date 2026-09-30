import { Hash, Settings2, UserRound } from 'lucide-react';
import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { ThisComputerStatus } from '@/features/desktop/ui/this-computer-status';
import { DirectSessionNavigation } from '@/features/direct-sessions/ui/direct-session-navigation';
import type { DirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import { RoomActions } from '@/features/room-directory/ui/room-actions';
import { AppDestinationLinks } from '@/shared/ui/app-navigation';

/**
 * 房间菜单：这个房间（回到房间对话、房间设置）、换个房间和新建房间、私聊；最下面是和顶栏一样的
 * 四个去处。语言、版本和安全都在“设置”里，这里不再重复。
 */
export function WorkspaceNavigation({
  currentCatalogId,
  activeDirectId,
  controller,
  onActivateRoom,
  onActivateDirect,
  onOpenRoomSettings,
  roomName,
  userName,
}: {
  readonly currentCatalogId?: string;
  readonly activeDirectId: string | null;
  readonly controller: DirectSessionController;
  readonly onActivateRoom: () => void;
  readonly onActivateDirect: (catalogId: string) => void;
  /** 没登录时没有房间设置。 */
  readonly onOpenRoomSettings: (() => void) | null;
  readonly roomName: string;
  readonly userName: string | null;
}) {
  const { t } = useTranslation();
  return (
    <div className="workspace-navigation">
      <Link className="workspace-navigation__brand" to="/rooms" aria-label={t('app.name')}>
        <img src="/agent-room-mark.svg" alt="" />
        <span>{t('app.name')}</span>
      </Link>
      <nav className="workspace-navigation__rooms" aria-label={t('roomWorkspace.navigation')}>
        <p className="workspace-navigation__label">{t('roomWorkspace.thisRoom')}</p>
        <button
          className="workspace-navigation__room"
          type="button"
          aria-pressed={activeDirectId === null}
          onClick={onActivateRoom}
        >
          <Hash aria-hidden="true" />
          <strong>{roomName}</strong>
        </button>
        {onOpenRoomSettings === null ? null : (
          <button
            className="workspace-navigation__link"
            type="button"
            aria-haspopup="dialog"
            onClick={onOpenRoomSettings}
          >
            <Settings2 aria-hidden="true" />
            {t('roomSettings.open')}
          </button>
        )}
        <RoomActions currentCatalogId={currentCatalogId} />
      </nav>
      <DirectSessionNavigation
        activeCatalogId={activeDirectId}
        controller={controller}
        onActivate={onActivateDirect}
      />
      <div className="workspace-navigation__footer">
        <AppDestinationLinks active="rooms" className="workspace-navigation__app" />
        <ThisComputerStatus />
        {userName === null ? null : (
          <p className="workspace-navigation__account">
            <UserRound aria-hidden="true" />
            <span>{userName}</span>
          </p>
        )}
      </div>
    </div>
  );
}
