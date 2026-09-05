import { Hash, Menu, UsersRound } from 'lucide-react';
import type { Ref } from 'react';
import { useTranslation } from 'react-i18next';

export type RoomBeaconProps = {
  readonly agentCount: number;
  readonly membersButtonRef: Ref<HTMLButtonElement>;
  readonly roomName: string;
  readonly topic?: string;
  readonly onOpenNavigation: () => void;
  readonly onOpenMembers: () => void;
};

export function RoomBeacon({
  agentCount,
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
        <span>{t('roomWorkspace.room')}</span>
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
        <span>{t('lobby.room.agents', { count: agentCount })}</span>
      </button>
    </header>
  );
}
