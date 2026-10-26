import { Banner, Button, Details, Spinner } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { Archive, ArrowRight, LockKeyhole, MailOpen, ShieldAlert } from 'lucide-react';
import { useId, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { RoomUnreadMark, useRoomUnread } from '@/features/inbox/ui/room-unread-mark';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { splitByOwner, type RoomOwner } from '@/features/room-directory/domain/my-private-rooms';
import { AccountIdCopy } from '@/features/room-directory/ui/account-id-copy';
import {
  useMyPrivateRooms,
  useRoomCommands,
  useRoomOwners,
  type RoomResult,
} from '@/features/room-directory/ui/use-rooms';

import './rooms.css';

/**
 * “房间”页上的“你的私人房间”：别人的邀请（加入或拒绝）、你建的房间、你加入的别人的房间（写着房主是谁），
 * 和收起来的已归档房间。
 */
export function MyRoomsSection() {
  const services = useOptionalAppServices();
  if (services === null) return null;
  return <ConnectedMyRooms />;
}

function ConnectedMyRooms() {
  const { t } = useTranslation();
  const mine = useMyPrivateRooms();
  const commands = useRoomCommands();
  const [pendingCatalogId, setPendingCatalogId] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const [failure, setFailure] = useState<string | null>(null);
  const busy = pendingCatalogId !== null;
  const joined = splitByOwner(mine.joined, mine.principalId);
  const archivedOthers = splitByOwner(mine.archived, mine.principalId).others;
  // 只问别人的房间：自己的房间不用写房主。
  const owners = useRoomOwners([...joined.others, ...archivedOthers]);

  const run = async (room: PrivateRoom, command: () => Promise<RoomResult>): Promise<void> => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPendingCatalogId(room.catalogId);
    setFailure(null);
    try {
      const result = await command();
      if (!result.ok) setFailure(result.error.code);
    } catch {
      setFailure('rooms.interrupted');
    } finally {
      pendingRef.current = false;
      setPendingCatalogId(null);
    }
  };

  return (
    <section aria-labelledby="my-rooms-title" className="my-rooms">
      <h2 id="my-rooms-title">{t('rooms.private')}</h2>
      {failure === null ? null : (
        <Banner tone="danger" title={t('rooms.actionFailed')}>
          <Details summary={t('rooms.details')}>
            <code>{failure}</code>
          </Details>
        </Banner>
      )}
      {mine.invitations.length === 0 ? null : (
        <ul aria-label={t('rooms.invitations')} className="my-rooms__invitations">
          {mine.invitations.map((room) => (
            <li className="room-row room-row--invitation" key={room.catalogId}>
              <span aria-hidden="true" className="room-row__icon">
                <MailOpen />
              </span>
              <span className="room-row__text">
                <small>{t('rooms.invitations')}</small>
                <strong>{room.name}</strong>
                {mine.serverReadable.has(room.catalogId) ? (
                  <small className="room-row__notice">
                    <ShieldAlert aria-hidden="true" />
                    {t('rooms.invitations.serverReadable')}
                  </small>
                ) : null}
              </span>
              <span className="room-row__actions">
                <Button
                  disabled={busy}
                  onClick={() => void run(room, async () => await commands.decline(room))}
                  size="compact"
                  tone="quiet"
                >
                  {t('rooms.decline')}
                </Button>
                <Button
                  disabled={busy}
                  icon={pendingCatalogId === room.catalogId ? <Spinner /> : undefined}
                  onClick={() => void run(room, async () => await commands.enter(room, true))}
                  size="compact"
                >
                  {t('rooms.join')}
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}
      {mine.state === 'loading' ? (
        <p className="my-rooms__state" role="status">
          <Spinner />
          {t('rooms.loading')}
        </p>
      ) : null}
      {mine.state === 'failed' ? (
        <Banner
          action={
            <Button onClick={mine.retry} size="compact" tone="quiet">
              {t('rooms.retry')}
            </Button>
          }
          tone="warning"
        >
          {t('rooms.loadFailed')}
        </Banner>
      ) : null}
      {mine.state === 'ready' &&
      mine.joined.length === 0 &&
      mine.invitations.length === 0 &&
      mine.archived.length === 0 ? (
        <p className="my-rooms__state">{t('rooms.noPrivate')}</p>
      ) : null}
      {joined.created.length === 0 ? null : (
        <RoomGroup title={t('rooms.created')}>
          {joined.created.map((room) => (
            <RoomCard key={room.catalogId} room={room} />
          ))}
        </RoomGroup>
      )}
      {joined.others.length === 0 ? null : (
        <RoomGroup title={t('rooms.joinedOthers')}>
          {joined.others.map((room) => (
            <RoomCard key={room.catalogId} owner={owners.get(room.catalogId)} room={room} />
          ))}
        </RoomGroup>
      )}
      {mine.archived.length === 0 ? null : (
        <Details
          className="my-rooms__archived"
          summary={t('rooms.archived', { count: mine.archived.length })}
        >
          <p>{t('rooms.archived.detail')}</p>
          <div className="my-rooms__grid">
            {mine.archived.map((room) => (
              <RoomCard
                archived
                key={room.catalogId}
                owner={owners.get(room.catalogId)}
                room={room}
              />
            ))}
          </div>
        </Details>
      )}
      {mine.principalId === undefined ? null : <AccountIdCopy principalId={mine.principalId} />}
    </section>
  );
}

/** 你建的、你加入的：各有一个小标题，下面是一样的房间卡片。 */
function RoomGroup({ title, children }: { readonly title: string; readonly children: ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="my-rooms__group">
      <h3 id={headingId}>{title}</h3>
      <div className="my-rooms__grid">{children}</div>
    </section>
  );
}

/**
 * 一间私人房间的卡片，点了就进去。归档了的换个图标：进去只能看之前的消息。别人的房间写着房主是谁；
 * 名字还没问到或没问成时先不写，房间照样列出来。
 */
function RoomCard({
  room,
  archived = false,
  owner,
}: {
  readonly room: PrivateRoom;
  readonly archived?: boolean;
  readonly owner?: RoomOwner | undefined;
}) {
  const { t } = useTranslation();
  const unread = useRoomUnread(room.matrixRoomId);
  return (
    <Link
      className={`room-row room-row--link room-row--card${archived ? ' room-row--archived' : ''}`}
      params={{ catalogId: room.catalogId, roomId: room.matrixRoomId }}
      search={{}}
      to="/lobby/$catalogId/instance/$roomId"
    >
      <span aria-hidden="true" className="room-row__icon">
        {archived ? <Archive /> : <LockKeyhole />}
      </span>
      <span className="room-row__text">
        <strong>{room.name}</strong>
        {owner === undefined ? null : (
          <small className="room-row__owner">
            {owner.kind === 'person'
              ? t('rooms.owner', { name: owner.name })
              : owner.name === null
                ? t('rooms.owner.networkAgentUnnamed')
                : t('rooms.owner.networkAgent', { name: owner.name })}
          </small>
        )}
        {room.description === '' ? null : <small>{room.description}</small>}
      </span>
      <RoomUnreadMark
        count={unread?.count}
        mentioned={unread?.mentioned ?? false}
        more={unread?.more ?? false}
      />
      <ArrowRight aria-hidden="true" className="room-row__mark" />
    </Link>
  );
}
