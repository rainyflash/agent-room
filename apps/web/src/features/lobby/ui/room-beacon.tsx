import { Link } from '@tanstack/react-router';
import { ArrowLeft, Hash, Settings2, UsersRound } from 'lucide-react';
import { useId, type ReactNode, type Ref } from 'react';
import { useTranslation } from 'react-i18next';

import { RoomKindLabel, type RoomKind } from './room-kind-label';

export type RoomBeaconProps = RoomKind & {
  readonly agentCount: number;
  /** “对话 / 场景”的切换，排在房间设置前面。 */
  readonly layoutSwitch?: ReactNode;
  readonly membersButtonRef: Ref<HTMLButtonElement>;
  readonly roomName: string;
  readonly topic?: string;
  /** 没登录时没有房间设置。 */
  readonly onOpenSettings: (() => void) | null;
  /** 公开房间里有几个申请在等你批准；齿轮上亮一个点。 */
  readonly pendingJoinRequests?: number;
  readonly onOpenMembers: () => void;
};

/** 房间页顶上：左上角回到房间列表，右边是房间设置和在场的 Agent。 */
export function RoomBeacon({
  agentCount,
  privateRoom,
  listed = false,
  unencrypted = false,
  serverReadable = false,
  archived = false,
  layoutSwitch,
  membersButtonRef,
  roomName,
  topic,
  onOpenSettings,
  onOpenMembers,
  pendingJoinRequests = 0,
}: RoomBeaconProps) {
  const { t } = useTranslation();
  const attentionId = useId();
  return (
    <header className="workspace-header">
      <Link
        className="workspace-header__back"
        to="/rooms"
        aria-label={t('roomWorkspace.backToRooms')}
        title={t('roomWorkspace.backToRooms')}
      >
        <ArrowLeft aria-hidden="true" />
      </Link>
      <div className="workspace-header__symbol" aria-hidden="true">
        <Hash />
      </div>
      <div className="workspace-header__identity">
        <RoomKindLabel
          archived={archived}
          listed={listed}
          privateRoom={privateRoom}
          serverReadable={serverReadable}
          unencrypted={unencrypted}
        />
        <h1>{roomName}</h1>
      </div>
      {topic === undefined ? null : <p className="workspace-header__topic">{topic}</p>}
      {layoutSwitch}
      {onOpenSettings === null ? null : (
        <button
          type="button"
          className="workspace-header__settings"
          aria-haspopup="dialog"
          aria-label={t('roomSettings.open')}
          aria-describedby={pendingJoinRequests > 0 ? attentionId : undefined}
          title={t('roomSettings.open')}
          onClick={onOpenSettings}
        >
          <Settings2 aria-hidden="true" />
          {pendingJoinRequests > 0 ? (
            <span className="workspace-header__attention" id={attentionId}>
              <span className="sr-only">
                {t('privateRooms.joinRequests.attention', { count: pendingJoinRequests })}
              </span>
            </span>
          ) : null}
        </button>
      )}
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
