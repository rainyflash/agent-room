import { Button, Details } from '@agent-room/ui-system';
import { LoaderCircle, RotateCw } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useRoomRoleChanges } from '@/features/private-rooms/data/private-room-queries';
import {
  roleChangeFacts,
  type RoleChangeFact,
  type RoomRoleChange,
  type RoomRoleChangeGateway,
} from '@/features/private-rooms/domain/room-role-changes';
import type { RoomRole } from '@/features/private-rooms/domain/room-roles';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';

import './room-role-changes.css';

/** 服务器一次给最近这么多条；到了这个数就说一句更早的这里看不到。 */
const SHOWN_AT_MOST = 100;

export type RoomRoleChangesProps = {
  readonly catalogId: string;
  readonly gateway: RoomRoleChangeGateway;
  /** 成员一节里用的那一套名字：问不到的写“名字暂时看不到”。 */
  readonly nameOf: (principalId: string) => string;
};

/**
 * 房间设置“成员”一节最下面的“身份变动”（`specs/redesign/room-roles.md` 的“记录”）：谁、什么时候、把谁、
 * 从什么改成了什么，新的在前。只给房主和管理员看，平时收着，点开了才去读。
 */
export function RoomRoleChanges({ catalogId, gateway, nameOf }: RoomRoleChangesProps) {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const query = useRoomRoleChanges(gateway, catalogId, open);
  const result = query.data;

  return (
    <Details
      className="room-role-changes"
      onToggle={setOpen}
      summary={t('privateRooms.roleChanges.title')}
    >
      <p className="room-role-changes__detail">{t('privateRooms.roleChanges.detail')}</p>
      {result === undefined ? (
        <p className="room-role-changes__state" role="status">
          <LoaderCircle aria-hidden="true" className="private-room-spin" />
          {t('privateRooms.roleChanges.loading')}
        </p>
      ) : !result.ok ? (
        <>
          <PrivateRoomFailureNotice failure={result.error} />
          <Button
            disabled={query.isFetching}
            icon={<RotateCw aria-hidden="true" />}
            onClick={() => {
              void query.refetch();
            }}
            size="compact"
            tone="ghost"
          >
            {t('privateRooms.roleChanges.retry')}
          </Button>
        </>
      ) : result.value.length === 0 ? (
        <p className="room-role-changes__state">{t('privateRooms.roleChanges.none')}</p>
      ) : (
        <>
          <ol className="room-role-changes__list">
            {result.value.map((change) => (
              <li key={change.id}>
                {roleChangeFacts(change).map((fact) => (
                  // 一条记录里同一种事只有一件，开关按是哪一个分。
                  <p key={fact.kind === 'switch' ? fact.capability : fact.kind}>
                    <FactSentence
                      fact={fact}
                      inSubRoom={change.subRoom !== undefined}
                      name={nameOf(change.targetPrincipalId)}
                    />
                  </p>
                ))}
                <small>{byline(change)}</small>
              </li>
            ))}
          </ol>
          {result.value.length >= SHOWN_AT_MOST ? (
            <p className="room-role-changes__state">
              {t('privateRooms.roleChanges.latest', { count: SHOWN_AT_MOST })}
            </p>
          ) : null}
        </>
      )}
    </Details>
  );

  function byline(change: RoomRoleChange): string {
    const time = formatDateTime(change.changedAtUnixMs, i18n.resolvedLanguage);
    const who =
      change.actorPrincipalId === undefined
        ? time
        : t('privateRooms.roleChanges.by', { actor: nameOf(change.actorPrincipalId), time });
    return change.subRoom === undefined
      ? who
      : `${who} · ${t('privateRooms.roleChanges.inSubRoom', { name: change.subRoom.name })}`;
  }
}

function FactSentence({
  fact,
  inSubRoom,
  name,
}: {
  readonly fact: RoleChangeFact;
  /** 子房间里身上有“管理”的是子房间管理员，不是整间房的管理员。 */
  readonly inSubRoom: boolean;
  readonly name: string;
}) {
  const { t } = useTranslation();
  const role = (value: RoomRole): string =>
    inSubRoom && value === 'admin'
      ? t('privateRooms.roleChanges.subRoomAdmin')
      : t(`privateRooms.role.${value}`);
  switch (fact.kind) {
    case 'role':
      return t('privateRooms.roleChanges.role', { from: role(fact.from), name, to: role(fact.to) });
    case 'entered':
      return t('privateRooms.roleChanges.entered', { name, role: role(fact.as) });
    case 'exited':
      return t(`privateRooms.roleChanges.exited.${fact.how}`, { name, role: role(fact.from) });
    case 'switch':
      return t(`privateRooms.roleChanges.${fact.capability}.${fact.on ? 'on' : 'off'}`, { name });
    case 'permissions':
      return t('privateRooms.roleChanges.permissions', { name });
  }
}
