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

/**
 * 这一小句是不是要一直提醒着的那种：归档了、服务器读得到、不加密。
 * 地方不够时先保住它：手机上的顶栏只写它的后一半（见 `Notice`），房间名先收。
 */
export function roomKindWarns({
  privateRoom,
  unencrypted = false,
  serverReadable = false,
  archived = false,
}: RoomKind): boolean {
  return privateRoom && (archived || serverReadable || unencrypted);
}

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
      <Notice
        text={t('roomWorkspace.privateRoomArchived')}
        title={t('roomWorkspace.archivedNotice')}
      />
    );
  if (privateRoom && serverReadable)
    return (
      <Notice
        text={t('roomWorkspace.privateRoomServerReadable')}
        title={t('roomWorkspace.serverReadableNotice')}
      />
    );
  if (privateRoom && unencrypted)
    return (
      <Notice
        text={t('roomWorkspace.publicRoomUnencrypted')}
        title={t('roomWorkspace.unencryptedNotice')}
      />
    );
  if (privateRoom && listed) return <span>{t('roomWorkspace.publicRoom')}</span>;
  if (privateRoom) return <span>{t('roomWorkspace.privateRoom')}</span>;
  return (
    <span title={t('roomWorkspace.publicNotice')}>{t('roomWorkspace.publicLobbyVisible')}</span>
  );
}

/** 这几句里，前一半（哪种房间）和后一半（要提醒的话）中间的那个点。两种语言的文案都这么写。 */
const noticeSeparator = ' · ';

/**
 * 要一直提醒着的一句，比如“私人房间 · 已归档”。前一半单独包一层（`.room-kind__what`）：
 * 地方不够时只收前一半，后一半“已归档”一直写全（手机上的顶栏，样式在 `room-desk.css`）。
 */
function Notice({ text, title }: { readonly text: string; readonly title: string }) {
  const cut = text.indexOf(noticeSeparator);
  const split = cut < 0 ? 0 : cut + noticeSeparator.length;
  return (
    <span className="workspace-header__notice" title={title}>
      {split === 0 ? null : <span className="room-kind__what">{text.slice(0, split)}</span>}
      {text.slice(split)}
    </span>
  );
}
