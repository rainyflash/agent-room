import { Avatar, Button, RoomGlyph } from '@agent-room/ui-system';
import { Check, LoaderCircle, RotateCcw } from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { useCatalogUnread } from '@/features/inbox/ui/room-unread-mark';
import { usePrivateRoomPeople } from '@/features/private-rooms/data/private-room-queries';
import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { subRoomCategoryName, subRoomRowAction } from '@/features/sub-rooms/domain/sub-room-browse';

import { FailureNote, RequestForm } from './sub-room-browser';
import { SubRoomLink } from './sub-room-link';
import { useSubRoomActions } from './use-sub-room-actions';
import './sub-room-door.css';

export type SubRoomDoorProps = {
  /** 这间子房间所在的整间房的名单：父房间和分类从这里来。 */
  readonly list: SubRoomList;
  readonly room: SubRoom;
};

/** 卡片上最多画几个管这一间的人的头像。 */
const SHOWN_MANAGERS = 3;

/**
 * 还没进的一间子房间（`specs/redesign/sub-rooms.md`“网页”，画布“有门的子房间”）：名字、用途、谁在管，
 * 和照你的处境给的一个动作：加入、申请加入（在等的能撤回）、只能请进来。归档了的写“已经归档”，
 * 能恢复的人有“恢复”。里面有谁、说了什么，进去以后才看得到。
 */
export function SubRoomDoor({ list, room }: SubRoomDoorProps) {
  const { t } = useTranslation();
  const { privateRooms } = useAppServices();
  const headingId = useId();
  const parentId = list.parent.catalogId;
  const actions = useSubRoomActions(parentId);
  const [requesting, setRequesting] = useState(false);
  const people = usePrivateRoomPeople(privateRooms, parentId, room.managerPrincipalIds.length > 0);
  const known = people.data?.ok === true ? people.data.value : [];
  const managers = room.managerPrincipalIds.flatMap((principalId) => {
    const person = known.find((entry) => entry.principalId === principalId);
    return person === undefined ? [] : [person.displayName];
  });
  const managerNames = managers.join(t('subRooms.create.listSeparator'));
  const archived = room.archivedAtUnixMs !== undefined;
  const action = subRoomRowAction(room);
  const category = subRoomCategoryName(list.categories, room);
  return (
    <section aria-labelledby={headingId} className="sub-room-door">
      <div className="sub-room-door__card">
        <p className="sub-room-door__kicker">
          <RoomGlyph kind={room.access === 'open' ? 'open' : 'gated'} />
          <span>
            {[
              category ?? t('subRooms.browse.uncategorized'),
              t(`subRooms.door.access.${room.access}`),
            ].join(' · ')}
          </span>
        </p>
        {/* 从左栏、名单打开时焦点落在这里（房间页放的），读屏接着读这一张。 */}
        <h2 id={headingId} tabIndex={-1}>
          {room.name}
        </h2>
        {room.description === '' ? null : (
          <p className="sub-room-door__purpose">{room.description}</p>
        )}
        <p className="sub-room-door__managers">
          {managers.length === 0 ? null : (
            <span aria-hidden="true" className="sub-room-door__faces">
              {managers.slice(0, SHOWN_MANAGERS).map((name) => (
                <Avatar key={name} tint="ink">
                  {(Array.from(name)[0] ?? '').toUpperCase()}
                </Avatar>
              ))}
            </span>
          )}
          {managers.length === 0
            ? t('subRooms.door.managedByRoom')
            : t('subRooms.door.managedBy', { count: managers.length, names: managerNames })}
        </p>
        {archived ? (
          <ArchivedDoor actions={actions} room={room} />
        ) : (
          <>
            <div className="sub-room-door__action">
              <DoorAction
                action={action}
                actions={actions}
                managerNames={managerNames}
                onRequest={() => {
                  setRequesting(true);
                }}
                requesting={requesting}
                room={room}
              />
            </div>
            {requesting && action === 'request' ? (
              <RequestForm
                busy={actions.busy?.kind === 'request'}
                onCancel={() => {
                  setRequesting(false);
                }}
                onSend={(message) => {
                  actions.request(room, message);
                }}
                room={room}
              />
            ) : null}
          </>
        )}
        <FailureNote actions={actions} room={room} />
        {/* 归档了的谁都进不去，“进去以后”那句不写。 */}
        {room.joined || archived ? null : (
          <p className="sub-room-door__foot">{t('subRooms.door.foot')}</p>
        )}
      </div>
    </section>
  );
}

function DoorAction({
  action,
  actions,
  managerNames,
  onRequest,
  requesting,
  room,
}: {
  readonly action: ReturnType<typeof subRoomRowAction>;
  readonly actions: ReturnType<typeof useSubRoomActions>;
  readonly managerNames: string;
  readonly onRequest: () => void;
  readonly requesting: boolean;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const busy = actions.busy?.catalogId === room.catalogId ? actions.busy.kind : null;
  const spinner = <LoaderCircle aria-hidden="true" className="sub-room-browser__spin" />;
  switch (action) {
    case 'joined':
      return <OpenRoom room={room} />;
    case 'join':
      return (
        <Button
          aria-busy={busy === 'join'}
          disabled={busy !== null}
          icon={busy === 'join' ? spinner : undefined}
          onClick={() => {
            actions.join(room);
          }}
        >
          {t('subRooms.door.join')}
        </Button>
      );
    case 'request':
      return (
        <>
          {requesting ? null : (
            <Button aria-expanded={false} disabled={busy !== null} onClick={onRequest}>
              {t('subRooms.browse.action.request')}
            </Button>
          )}
          <p className="sub-room-door__hint">
            {managerNames === ''
              ? t('subRooms.door.approvedByRoom')
              : t('subRooms.door.approvedBy', { names: managerNames })}
          </p>
        </>
      );
    case 'pending':
      return (
        <>
          <p className="sub-room-door__state" role="status">
            {managerNames === ''
              ? t('subRooms.door.waitingForRoom')
              : t('subRooms.door.waitingFor', { names: managerNames })}
          </p>
          <Button
            aria-busy={busy === 'withdraw'}
            disabled={busy !== null}
            icon={busy === 'withdraw' ? spinner : undefined}
            onClick={() => {
              actions.withdraw(room);
            }}
            tone="ghost"
          >
            {t('subRooms.door.withdraw')}
          </Button>
        </>
      );
    case 'invite_only':
      return <p className="sub-room-door__state">{t('subRooms.door.inviteOnly')}</p>;
    case 'banned':
      return <p className="sub-room-door__state">{t('subRooms.failure.banned')}</p>;
  }
}

/** 归档了的：原来在里面的能打开看，管得了它的人能恢复。 */
function ArchivedDoor({
  actions,
  room,
}: {
  readonly actions: ReturnType<typeof useSubRoomActions>;
  readonly room: SubRoom;
}) {
  const { t } = useTranslation();
  const busy = actions.busy?.catalogId === room.catalogId ? actions.busy.kind : null;
  return (
    <div className="sub-room-door__action">
      <p className="sub-room-door__state">{t('subRooms.door.archived')}</p>
      {room.joined ? <OpenRoom room={room} /> : null}
      {room.canRestore === true ? (
        <Button
          aria-busy={busy === 'restore'}
          disabled={busy !== null}
          icon={
            busy === 'restore' ? (
              <LoaderCircle aria-hidden="true" className="sub-room-browser__spin" />
            ) : (
              <RotateCcw aria-hidden="true" />
            )
          }
          onClick={() => {
            actions.restore(room);
          }}
          tone="ghost"
        >
          {t('subRooms.browse.action.restore')}
        </Button>
      ) : null}
    </div>
  );
}

function OpenRoom({ room }: { readonly room: SubRoom }) {
  const { t } = useTranslation();
  const found = useCatalogUnread(room.catalogId);
  return (
    <>
      {room.archivedAtUnixMs === undefined ? (
        <p className="sub-room-door__state">
          <Check aria-hidden="true" />
          {t('subRooms.door.joined')}
        </p>
      ) : null}
      <SubRoomLink
        catalogId={room.catalogId}
        className="ar-button ar-button--primary ar-button--default"
        matrixRoomId={found?.roomId}
      >
        {t('subRooms.browse.action.open')}
      </SubRoomLink>
    </>
  );
}
