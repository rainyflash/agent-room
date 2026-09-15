import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { ArrowRight, Check, ChevronDown, Globe2, LockKeyhole, Plus, X } from 'lucide-react';
import { Button } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';
import { useAppServices, useOptionalAppServices } from '@/app/app-services';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  usePrivateRoomList,
  privateRoomListQueryKey,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  PrivateRoom,
  CreatePrivateRoomInput,
} from '@/features/private-rooms/domain/private-room';
import { usePublicRoomDirectory } from '../data/public-room-directory-query';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import { useOverlayContainer } from '@/shared/ui/overlay-container';
import './hall-actions.css';

export function HallActions({ currentCatalogId }: { readonly currentCatalogId?: string }) {
  const services = useOptionalAppServices();
  const [mode, setMode] = useState<'browse' | 'create' | null>(null);
  const { t } = useTranslation();
  if (services === null) return null;
  return (
    <div className="hall-actions">
      <Button
        size="compact"
        tone="quiet"
        icon={<ChevronDown aria-hidden="true" />}
        onClick={() => {
          setMode('browse');
        }}
      >
        {t('halls.switch')}
      </Button>
      <Button
        size="compact"
        icon={<Plus aria-hidden="true" />}
        onClick={() => {
          setMode('create');
        }}
      >
        {t('halls.create')}
      </Button>
      {mode === null ? null : (
        <HallDialog
          initialMode={mode}
          currentCatalogId={currentCatalogId}
          onClose={() => {
            setMode(null);
          }}
        />
      )}
    </div>
  );
}

function useHalls() {
  const services = useAppServices();
  const principalId = useOptionalSession()?.snapshot.context.principal?.principalId;
  const rooms = usePrivateRoomList(services.privateRooms, principalId);
  const visible = rooms.data?.ok
    ? rooms.data.value.filter(
        (room) =>
          room.status === 'active' &&
          room.members.some(
            (member) =>
              member.principalId === principalId && ['joined', 'invited'].includes(member.status),
          ),
      )
    : [];
  return { rooms, visible, principalId };
}

export function HallDirectorySection() {
  const { t } = useTranslation();
  const { rooms, visible, principalId } = useHalls();
  const joined = visible.filter((room) =>
    room.members.some((member) => member.principalId === principalId && member.status === 'joined'),
  );
  return (
    <section className="hall-directory-section" aria-label={t('halls.mine')}>
      <h2>{t('halls.mine')}</h2>
      {rooms.isPending ? <p role="status">{t('halls.loading')}</p> : null}
      {rooms.isError || rooms.data?.ok === false ? (
        <p role="alert">
          {t('halls.loadFailed')}{' '}
          <button type="button" onClick={() => void rooms.refetch()}>
            {t('halls.retry')}
          </button>
        </p>
      ) : null}
      {rooms.data?.ok && visible.length === 0 ? <p>{t('halls.empty')}</p> : null}
      <div className="hall-directory-section__rooms">
        {joined.map((room) => (
          <Link
            className="hall-directory-card"
            key={room.catalogId}
            to="/lobby/$catalogId/instance/$roomId"
            params={{ catalogId: room.catalogId, roomId: room.matrixRoomId }}
            search={{}}
          >
            <LockKeyhole aria-hidden="true" />
            <span>
              <strong>{room.name}</strong>
              <small>{t('halls.private')}</small>
            </span>
            <ArrowRight aria-hidden="true" />
          </Link>
        ))}
      </div>
    </section>
  );
}

function HallDialog({
  initialMode,
  currentCatalogId,
  onClose,
}: {
  readonly initialMode: 'browse' | 'create';
  readonly currentCatalogId: string | undefined;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation();
  const services = useAppServices();
  const { rooms, visible, principalId } = useHalls();
  const publicRooms = usePublicRoomDirectory(services.roomDirectory);
  const coordinator = useMemo(
    () => new PrivateRoomCoordinator(services.privateRooms, services.privateRoomMatrix),
    [services],
  );
  const navigate = useNavigate();
  const queries = useQueryClient();
  const overlay = useOverlayContainer();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [mode, setMode] = useState(initialMode);
  const [search, setSearch] = useState('');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [failure, setFailure] = useState<string | null>(null);
  const attempt = useRef<{ readonly id: string; readonly input: CreatePrivateRoomInput } | null>(
    null,
  );
  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  const enter = async (room: PrivateRoom) => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(true);
    setFailure(null);
    try {
      const invited = room.members.some(
        (member) => member.principalId === principalId && member.status === 'invited',
      );
      const result = await (invited ? coordinator.accept(room) : coordinator.open(room));
      if (!result.ok) {
        setFailure(result.error.code);
        return;
      }
      await queries.invalidateQueries({ queryKey: privateRoomListQueryKey });
      await navigate({
        to: '/lobby/$catalogId/instance/$roomId',
        params: { catalogId: room.catalogId, roomId: room.matrixRoomId },
        search: {},
      });
      onClose();
    } catch {
      setFailure('halls.open_failed');
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };
  const create = async () => {
    if (pendingRef.current || name.trim().length === 0 || principalId === undefined) return;
    pendingRef.current = true;
    setPending(true);
    setFailure(null);
    attempt.current ??= {
      id: new BrowserUuidV7Factory().next(),
      input: { name: name.trim(), description: description.trim(), invitations: [] },
    };
    try {
      // Keep the same ID and input after an ambiguous response or a failed Matrix join.
      const result = await coordinator.createAndJoin(attempt.current.id, attempt.current.input);
      await queries.invalidateQueries({ queryKey: privateRoomListQueryKey });
      if (!result.ok) {
        setFailure(result.error.code);
        return;
      }
      await navigate({
        to: '/lobby/$catalogId/instance/$roomId',
        params: { catalogId: result.value.catalogId, roomId: result.value.matrixRoomId },
        search: {},
      });
      onClose();
    } catch {
      setFailure('halls.create_failed');
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };
  const term = search.trim().toLocaleLowerCase();
  const matches = (room: { readonly name: string; readonly description: string }) =>
    `${room.name} ${room.description}`.toLocaleLowerCase().includes(term);
  const publicList = publicRooms.data?.ok ? publicRooms.data.value.filter(matches) : [];
  const mine = visible.filter(matches);
  return createPortal(
    <dialog
      ref={dialog}
      className="hall-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => {
        if (pending) event.preventDefault();
        else onClose();
      }}
    >
      <header>
        <div>
          <small>{t('app.name')}</small>
          <h2 id={titleId}>{t(mode === 'create' ? 'halls.create' : 'halls.switch')}</h2>
        </div>
        <button type="button" disabled={pending} onClick={onClose} aria-label={t('halls.close')}>
          <X aria-hidden="true" />
        </button>
      </header>
      {mode === 'create' ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void create();
          }}
        >
          <p>{t('halls.createHint')}</p>
          <label>
            {t('halls.name')}
            <input
              autoFocus
              required
              maxLength={128}
              value={name}
              disabled={pending || attempt.current !== null}
              onChange={(event) => {
                setName(event.target.value);
              }}
            />
          </label>
          <label>
            {t('halls.description')}
            <textarea
              rows={3}
              maxLength={1000}
              value={description}
              disabled={pending || attempt.current !== null}
              onChange={(event) => {
                setDescription(event.target.value);
              }}
            />
          </label>
          <p className="hall-dialog__privacy">
            <LockKeyhole aria-hidden="true" />
            {t('halls.privacy')}
          </p>
          <Button
            type="submit"
            disabled={pending || name.trim().length === 0 || principalId === undefined}
            icon={<Plus aria-hidden="true" />}
          >
            {t(pending ? 'halls.creating' : failure !== null ? 'halls.retry' : 'halls.createEnter')}
          </Button>
        </form>
      ) : (
        <>
          <div className="hall-dialog__tools">
            <input
              type="search"
              autoFocus
              aria-label={t('halls.search')}
              placeholder={t('halls.search')}
              value={search}
              onChange={(event) => {
                setSearch(event.target.value);
              }}
            />
            <Button
              disabled={pending}
              size="compact"
              onClick={() => {
                setMode('create');
                setFailure(null);
              }}
              icon={<Plus aria-hidden="true" />}
            >
              {t('halls.create')}
            </Button>
          </div>
          <h3>{t('halls.mine')}</h3>
          {rooms.isPending ? <p role="status">{t('halls.loading')}</p> : null}
          {rooms.isError || rooms.data?.ok === false ? (
            <p role="alert">
              {t('halls.loadFailed')}{' '}
              <button type="button" onClick={() => void rooms.refetch()}>
                {t('halls.retry')}
              </button>
            </p>
          ) : null}
          {mine.map((room) => (
            <button
              className="hall-dialog__row"
              type="button"
              key={room.catalogId}
              disabled={pending || room.catalogId === currentCatalogId}
              onClick={() => void enter(room)}
            >
              <LockKeyhole aria-hidden="true" />
              <span>
                <strong>{room.name}</strong>
                <small>
                  {t(
                    room.members.some(
                      (member) => member.principalId === principalId && member.status === 'invited',
                    )
                      ? 'halls.invited'
                      : 'halls.private',
                  )}
                </small>
              </span>
              {room.catalogId === currentCatalogId ? (
                <Check aria-label={t('halls.current')} />
              ) : (
                <ArrowRight aria-hidden="true" />
              )}
            </button>
          ))}
          {rooms.data?.ok && mine.length === 0 ? (
            <p>{t(term ? 'halls.noResults' : 'halls.empty')}</p>
          ) : null}
          <h3>{t('halls.public')}</h3>
          {publicRooms.isPending ? <p role="status">{t('halls.loading')}</p> : null}
          {publicRooms.isError || publicRooms.data?.ok === false ? (
            <p role="alert">
              {t('halls.loadFailed')}{' '}
              <button type="button" onClick={() => void publicRooms.refetch()}>
                {t('halls.retry')}
              </button>
            </p>
          ) : null}
          {publicList.map((room) => (
            <Link
              className="hall-dialog__row"
              key={room.catalogId}
              to="/lobby/$catalogId"
              params={{ catalogId: room.catalogId }}
              search={{}}
              onClick={onClose}
            >
              <Globe2 aria-hidden="true" />
              <span>
                <strong>{room.name}</strong>
                <small>{room.description}</small>
              </span>
              {room.catalogId === currentCatalogId ? (
                <Check aria-label={t('halls.current')} />
              ) : (
                <ArrowRight aria-hidden="true" />
              )}
            </Link>
          ))}
          {publicRooms.data?.ok && publicList.length === 0 ? <p>{t('halls.noResults')}</p> : null}
        </>
      )}
      {failure !== null ? (
        <div className="hall-dialog__failure" role="alert">
          <p>{t('halls.actionFailed')}</p>
          <details>
            <summary>{t('halls.details')}</summary>
            <code>{failure}</code>
          </details>
        </div>
      ) : null}
    </dialog>,
    overlay,
  );
}
