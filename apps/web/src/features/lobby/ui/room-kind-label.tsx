import { useTranslation } from 'react-i18next';

export type RoomKind = {
  /**
   * 有房间号的房间（私人房间、公开房间）还是公共大厅。公共大厅不登录也能看（/watch），
   * 一直提醒着。
   */
  readonly privateRoom: boolean;
  /** 公开了的房间：写“公开房间”，不写“私人房间”。 */
  readonly listed?: boolean;
  /** 不加密的公开房间：进来的人和服务器都读得到，一直提醒着。 */
  readonly unencrypted?: boolean;
  /** 网络 Agent 建的私人房间：服务器代它收发，读得到这里所有的消息，同样一直提醒着。 */
  readonly serverReadable?: boolean;
  /** 归档了的私人房间：之前的消息照样看，谁都不能再说话。比“服务器能读到”先说。 */
  readonly archived?: boolean;
};

/** 房间名旁边那一小句：这是哪种房间、谁读得到。场景和对话两种版面的顶栏共用。 */
export function RoomKindLabel({
  privateRoom,
  listed = false,
  unencrypted = false,
  serverReadable = false,
  archived = false,
}: RoomKind) {
  const { t } = useTranslation();
  if (privateRoom && archived)
    return (
      <span className="workspace-header__notice" title={t('roomWorkspace.archivedNotice')}>
        {t('roomWorkspace.privateRoomArchived')}
      </span>
    );
  if (privateRoom && serverReadable)
    return (
      <span className="workspace-header__notice" title={t('roomWorkspace.serverReadableNotice')}>
        {t('roomWorkspace.privateRoomServerReadable')}
      </span>
    );
  if (privateRoom && unencrypted)
    return (
      <span className="workspace-header__notice" title={t('roomWorkspace.unencryptedNotice')}>
        {t('roomWorkspace.publicRoomUnencrypted')}
      </span>
    );
  if (privateRoom && listed) return <span>{t('roomWorkspace.publicRoom')}</span>;
  if (privateRoom) return <span>{t('roomWorkspace.privateRoom')}</span>;
  return (
    <span title={t('roomWorkspace.publicNotice')}>{t('roomWorkspace.publicLobbyVisible')}</span>
  );
}
