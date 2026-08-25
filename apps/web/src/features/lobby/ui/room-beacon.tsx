import { Radio, UsersRound } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';

export type RoomBeaconProps = {
  readonly actions?: ReactNode;
  readonly agentCount: number;
  readonly catalogId: string;
  readonly roomName: string;
  readonly topic?: string;
};

export function RoomBeacon({ actions, agentCount, catalogId, roomName, topic }: RoomBeaconProps) {
  const { t } = useTranslation();
  return (
    <header className="room-beacon">
      <a aria-label={t('app.name')} className="room-beacon__brand" href="/connect">
        <img alt="" src="/agent-room-mark.svg" />
      </a>
      <div className="room-beacon__identity">
        <span>{catalogId}</span>
        <strong>{roomName}</strong>
        {topic === undefined ? null : <p>{topic}</p>}
      </div>
      <div className="room-beacon__presence" title={t('lobby.room.live')}>
        <Radio aria-hidden="true" />
        <span>{t('lobby.room.live')}</span>
      </div>
      <div className="room-beacon__count">
        <UsersRound aria-hidden="true" />
        <span>{t('lobby.room.agents', { count: agentCount })}</span>
      </div>
      {actions === undefined ? null : <div className="room-beacon__actions">{actions}</div>}
      <LanguageControl />
    </header>
  );
}
