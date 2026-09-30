import { Banner, Button, Details, Dialog, Spinner } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Globe2, LockKeyhole, MailOpen, Plus, Search } from 'lucide-react';
import { useId, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { usePublicRoomDirectory } from '@/features/room-directory/data/public-room-directory-query';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { AccountIdCopy } from '@/features/room-directory/ui/account-id-copy';
import {
  useMyPrivateRooms,
  useRoomCommands,
  type RoomListState,
  type RoomResult,
} from '@/features/room-directory/ui/use-rooms';

export type RoomSwitcherDialogProps = {
  readonly currentCatalogId?: string | undefined;
  readonly onClose: () => void;
  readonly onCreate: () => void;
};

/** 换个房间：连着服务，进房间、接受或拒绝邀请都在这里做完。 */
export function RoomSwitcherDialog({
  currentCatalogId,
  onClose,
  onCreate,
}: RoomSwitcherDialogProps) {
  const { roomDirectory } = useAppServices();
  const mine = useMyPrivateRooms();
  const directory = usePublicRoomDirectory(roomDirectory);
  const commands = useRoomCommands();
  const [pendingCatalogId, setPendingCatalogId] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const [failure, setFailure] = useState<string | null>(null);

  const run = async (
    room: PrivateRoom,
    command: () => Promise<RoomResult>,
    closeOnSuccess: boolean,
  ): Promise<void> => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPendingCatalogId(room.catalogId);
    setFailure(null);
    try {
      const result = await command();
      if (!result.ok) {
        setFailure(result.error.code);
        return;
      }
      if (closeOnSuccess) onClose();
    } catch {
      setFailure('rooms.interrupted');
    } finally {
      pendingRef.current = false;
      setPendingCatalogId(null);
    }
  };

  return (
    <RoomSwitcherView
      currentCatalogId={currentCatalogId}
      failure={failure}
      invitations={mine.invitations}
      joined={mine.joined}
      onClose={onClose}
      onCreate={onCreate}
      onDecline={(room) => void run(room, async () => await commands.decline(room), false)}
      onEnter={(room, invited) =>
        void run(room, async () => await commands.enter(room, invited), true)
      }
      onRetryPrivate={mine.retry}
      onRetryPublic={() => void directory.refetch()}
      pendingCatalogId={pendingCatalogId}
      principalId={mine.principalId}
      privateState={mine.state}
      publicRooms={directory.data?.ok === true ? directory.data.value : []}
      publicState={
        directory.isPending
          ? 'loading'
          : directory.isError || !directory.data.ok
            ? 'failed'
            : 'ready'
      }
    />
  );
}

export type RoomSwitcherViewProps = {
  readonly currentCatalogId?: string | undefined;
  readonly principalId: string | undefined;
  readonly invitations: readonly PrivateRoom[];
  readonly joined: readonly PrivateRoom[];
  readonly privateState: RoomListState;
  readonly onRetryPrivate: () => void;
  readonly publicRooms: readonly PublicRoomSummary[];
  readonly publicState: RoomListState;
  readonly onRetryPublic: () => void;
  /** 正在进入、接受或拒绝的房间；这时其他操作先停着。 */
  readonly pendingCatalogId: string | null;
  readonly failure: string | null;
  readonly onEnter: (room: PrivateRoom, invited: boolean) => void;
  readonly onDecline: (room: PrivateRoom) => void;
  readonly onCreate: () => void;
  readonly onClose: () => void;
};

/**
 * 换个房间：别人的邀请在最上面（加入或拒绝），然后是你的私人房间和公共大厅，一个搜索框
 * 过滤全部。底部是“新建房间”。
 */
export function RoomSwitcherView({
  currentCatalogId,
  principalId,
  invitations,
  joined,
  privateState,
  onRetryPrivate,
  publicRooms,
  publicState,
  onRetryPublic,
  pendingCatalogId,
  failure,
  onEnter,
  onDecline,
  onCreate,
  onClose,
}: RoomSwitcherViewProps) {
  const { t } = useTranslation();
  const [search, setSearch] = useState('');
  const term = search.trim().toLocaleLowerCase();
  const matches = (room: { readonly name: string; readonly description: string }) =>
    `${room.name} ${room.description}`.toLocaleLowerCase().includes(term);
  const busy = pendingCatalogId !== null;
  const visibleInvitations = invitations.filter(matches);
  const visibleJoined = joined.filter(matches);
  const visiblePublic = publicRooms.filter(matches);

  return (
    <Dialog
      className="room-switcher"
      closeLabel={t('rooms.close')}
      footer={
        <Button disabled={busy} icon={<Plus aria-hidden="true" />} onClick={onCreate}>
          {t('rooms.new')}
        </Button>
      }
      onClose={() => {
        if (!busy) onClose();
      }}
      title={t('rooms.switch')}
    >
      <label className="room-switcher__search">
        <Search aria-hidden="true" />
        <input
          aria-label={t('rooms.search')}
          autoFocus
          onChange={(event) => {
            setSearch(event.target.value);
          }}
          placeholder={t('rooms.search')}
          type="search"
          value={search}
        />
      </label>

      {failure === null ? null : (
        <Banner tone="danger" title={t('rooms.actionFailed')}>
          <Details summary={t('rooms.details')}>
            <code>{failure}</code>
          </Details>
        </Banner>
      )}

      {visibleInvitations.length === 0 ? null : (
        <RoomSection detail={t('rooms.invitations.detail')} title={t('rooms.invitations')}>
          {visibleInvitations.map((room) => (
            <li className="room-row room-row--invitation" key={room.catalogId}>
              <span aria-hidden="true" className="room-row__icon">
                <MailOpen />
              </span>
              <RoomText description={room.description} name={room.name} />
              <span className="room-row__actions">
                <Button
                  disabled={busy}
                  onClick={() => {
                    onDecline(room);
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t('rooms.decline')}
                </Button>
                <Button
                  disabled={busy}
                  icon={pendingCatalogId === room.catalogId ? <Spinner /> : undefined}
                  onClick={() => {
                    onEnter(room, true);
                  }}
                  size="compact"
                >
                  {t('rooms.join')}
                </Button>
              </span>
            </li>
          ))}
        </RoomSection>
      )}

      <RoomSection title={t('rooms.private')}>
        {visibleJoined.map((room) => {
          const current = room.catalogId === currentCatalogId;
          return (
            <li key={room.catalogId}>
              <button
                aria-current={current ? 'page' : undefined}
                className="room-row room-row--link"
                disabled={busy || current}
                onClick={() => {
                  onEnter(room, false);
                }}
                type="button"
              >
                <span aria-hidden="true" className="room-row__icon">
                  <LockKeyhole />
                </span>
                <RoomText description={room.description} name={room.name} />
                <RoomMark current={current} pending={pendingCatalogId === room.catalogId} />
              </button>
            </li>
          );
        })}
        <RoomListFooter
          empty={visibleJoined.length === 0 && visibleInvitations.length === 0}
          emptyText={term === '' ? t('rooms.noPrivate') : t('rooms.noResults')}
          onRetry={onRetryPrivate}
          state={privateState}
        />
      </RoomSection>

      <RoomSection title={t('rooms.public')}>
        {visiblePublic.map((room) => {
          const current = room.catalogId === currentCatalogId;
          return (
            <li key={room.catalogId}>
              <Link
                aria-current={current ? 'page' : undefined}
                className="room-row room-row--link"
                onClick={onClose}
                params={{ catalogId: room.catalogId }}
                search={{}}
                to="/lobby/$catalogId"
              >
                <span aria-hidden="true" className="room-row__icon">
                  <Globe2 />
                </span>
                <RoomText description={room.description} name={room.name} />
                <RoomMark current={current} pending={false} />
              </Link>
            </li>
          );
        })}
        <RoomListFooter
          empty={visiblePublic.length === 0}
          emptyText={t('rooms.noResults')}
          onRetry={onRetryPublic}
          state={publicState}
        />
      </RoomSection>

      {principalId === undefined ? null : <AccountIdCopy principalId={principalId} />}
    </Dialog>
  );
}

function RoomSection({
  title,
  detail,
  children,
}: {
  readonly title: string;
  readonly detail?: string;
  readonly children: ReactNode;
}) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="room-switcher__section">
      <h3 id={headingId}>{title}</h3>
      {detail === undefined ? null : <p className="room-switcher__detail">{detail}</p>}
      <ul className="room-switcher__list">{children}</ul>
    </section>
  );
}

function RoomText({ name, description }: { readonly name: string; readonly description: string }) {
  return (
    <span className="room-row__text">
      <strong>{name}</strong>
      {description === '' ? null : <small>{description}</small>}
    </span>
  );
}

function RoomMark({ current, pending }: { readonly current: boolean; readonly pending: boolean }) {
  const { t } = useTranslation();
  if (pending) return <Spinner className="room-row__mark" />;
  if (current) return <span className="room-row__here">{t('rooms.current')}</span>;
  return <ArrowRight aria-hidden="true" className="room-row__mark" />;
}

/** 列表下面的一行：读取中、读取失败（带重试）、或者空的说明。放在 `<ul>` 里所以用 `<li>`。 */
function RoomListFooter({
  state,
  empty,
  emptyText,
  onRetry,
}: {
  readonly state: RoomListState;
  readonly empty: boolean;
  readonly emptyText: string;
  readonly onRetry: () => void;
}) {
  const { t } = useTranslation();
  if (state === 'loading') {
    return (
      <li className="room-switcher__state" role="status">
        <Spinner />
        {t('rooms.loading')}
      </li>
    );
  }
  if (state === 'failed') {
    return (
      <li className="room-switcher__state">
        <Banner
          action={
            <Button onClick={onRetry} size="compact" tone="quiet">
              {t('rooms.retry')}
            </Button>
          }
          tone="warning"
        >
          {t('rooms.loadFailed')}
        </Banner>
      </li>
    );
  }
  return empty ? <li className="room-switcher__state">{emptyText}</li> : null;
}
