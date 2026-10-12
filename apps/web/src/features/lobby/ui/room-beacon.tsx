import { Hash, Menu, UsersRound } from 'lucide-react';
import type { Ref } from 'react';
import { useTranslation } from 'react-i18next';

export type RoomBeaconProps = {
  readonly agentCount: number;
  /** 私人房间还是公共大厅，写在房间名上面。公共大厅不登录也能看（/watch），一直提醒着。 */
  readonly privateRoom: boolean;
  /** 网络 Agent 建的私人房间：服务器代它收发，读得到这里所有的消息，同样一直提醒着。 */
  readonly serverReadable?: boolean;
  /** 归档了的私人房间：之前的消息照样看，谁都不能再说话。比“服务器能读到”先说。 */
  readonly archived?: boolean;
  readonly membersButtonRef: Ref<HTMLButtonElement>;
  readonly roomName: string;
  readonly topic?: string;
  readonly onOpenNavigation: () => void;
  readonly onOpenMembers: () => void;
};

export function RoomBeacon({
  agentCount,
  privateRoom,
  serverReadable = false,
  archived = false,
  membersButtonRef,
  roomName,
  topic,
  onOpenNavigation,
  onOpenMembers,
}: RoomBeaconProps) {
  const { t } = useTranslation();
  return (
    <header className="workspace-header">
      <button
        className="workspace-header__menu"
        type="button"
        aria-label={t('roomWorkspace.openNavigation')}
        onClick={onOpenNavigation}
      >
        <Menu aria-hidden="true" />
      </button>
      <div className="workspace-header__symbol" aria-hidden="true">
        <Hash />
      </div>
      <div className="workspace-header__identity">
        {privateRoom && archived ? (
          <span className="workspace-header__notice" title={t('roomWorkspace.archivedNotice')}>
            {t('roomWorkspace.privateRoomArchived')}
          </span>
        ) : privateRoom && serverReadable ? (
          <span
            className="workspace-header__notice"
            title={t('roomWorkspace.serverReadableNotice')}
          >
            {t('roomWorkspace.privateRoomServerReadable')}
          </span>
        ) : privateRoom ? (
          <span>{t('roomWorkspace.privateRoom')}</span>
        ) : (
          <span title={t('roomWorkspace.publicNotice')}>
            {t('roomWorkspace.publicLobbyVisible')}
          </span>
        )}
        <h1>{roomName}</h1>
      </div>
      {topic === undefined ? null : <p className="workspace-header__topic">{topic}</p>}
      <button
        type="button"
        className="workspace-header__members"
        ref={membersButtonRef}
        aria-label={t('roomWorkspace.openMembers')}
        onClick={onOpenMembers}
      >
        <UsersRound aria-hidden="true" />
        <span className="workspace-header__members-label">
          {t('studio.presentCount', { count: agentCount })}
        </span>
        <span className="workspace-header__members-count" aria-hidden="true">
          {agentCount}
        </span>
      </button>
    </header>
  );
}
