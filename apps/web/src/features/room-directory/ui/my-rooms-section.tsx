import { Banner, Button, Details, Spinner } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { ArrowRight, LockKeyhole, MailOpen } from 'lucide-react';
import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { AccountIdCopy } from '@/features/room-directory/ui/account-id-copy';
import {
  useMyPrivateRooms,
  useRoomCommands,
  type RoomResult,
} from '@/features/room-directory/ui/use-rooms';

import './rooms.css';

/** “房间”页上的“你的私人房间”：别人的邀请（加入或拒绝）和已经加入的房间。 */
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
      {mine.state === 'ready' && mine.joined.length === 0 && mine.invitations.length === 0 ? (
        <p className="my-rooms__state">{t('rooms.noPrivate')}</p>
      ) : null}
      {mine.joined.length === 0 ? null : (
        <div className="my-rooms__grid">
          {mine.joined.map((room) => (
            <Link
              className="room-row room-row--link room-row--card"
              key={room.catalogId}
              params={{ catalogId: room.catalogId, roomId: room.matrixRoomId }}
              search={{}}
              to="/lobby/$catalogId/instance/$roomId"
            >
              <span aria-hidden="true" className="room-row__icon">
                <LockKeyhole />
              </span>
              <span className="room-row__text">
                <strong>{room.name}</strong>
                {room.description === '' ? null : <small>{room.description}</small>}
              </span>
              <ArrowRight aria-hidden="true" className="room-row__mark" />
            </Link>
          ))}
        </div>
      )}
      {mine.principalId === undefined ? null : <AccountIdCopy principalId={mine.principalId} />}
    </section>
  );
}
