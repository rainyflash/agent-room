import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  ArrowRight,
  Check,
  DoorOpen,
  LoaderCircle,
  LockKeyhole,
  Settings2,
  UserRoundCheck,
  X,
} from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  privateRoomListQueryKey,
  usePrivateRoomList,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  CreatePrivateRoomInput,
  PrivateRoom,
  PrivateRoomFailure,
} from '@/features/private-rooms/domain/private-room';
import {
  PrivateRoomCreateFlow,
  PrivateRoomFailureNotice,
} from '@/features/private-rooms/ui/private-room-create-flow';
import { PrivateRoomGovernance } from '@/features/private-rooms/ui/private-room-governance';
import type { WebSession } from '@/features/session/domain/session';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { Result } from '@/shared/result';
import { useAppServices } from '@/app/app-services';

export type PrivateRoomHubProps = {
  readonly currentCatalogId: string;
  readonly onEnterRoom: (catalogId: string, matrixRoomId: string) => void;
  readonly onExitRoom: () => void;
  readonly principal: WebSession;
};

type HubView = 'create' | 'governance' | 'rooms';

type RoomCommand = {
  readonly execute: () => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  readonly onSuccess?: (room: PrivateRoom) => void;
};

export function PrivateRoomHub({
  currentCatalogId,
  onEnterRoom,
  onExitRoom,
  principal,
}: PrivateRoomHubProps) {
  const { t } = useTranslation();
  const { privateRoomMatrix, privateRooms } = useAppServices();
  const queryClient = useQueryClient();
  const list = usePrivateRoomList(privateRooms);
  const coordinator = useMemo(
    () => new PrivateRoomCoordinator(privateRooms, privateRoomMatrix),
    [privateRoomMatrix, privateRooms],
  );
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const [open, setOpen] = useState(false);
  const [view, setView] = useState<HubView>('rooms');
  const [failure, setFailure] = useState<PrivateRoomFailure | null>(null);
  const rooms = list.data?.ok === true ? list.data.value : [];
  const currentRoom = rooms.find((room) => room.catalogId === currentCatalogId) ?? null;
  const pendingInvitations = rooms.filter(
    (room) =>
      room.members.find((member) => member.principalId === principal.principalId)?.status ===
      'invited',
  ).length;
  const mutation = useMutation({
    mutationFn: async (command: RoomCommand) => await command.execute(),
    onSuccess: async (result, command) => {
      if (!result.ok) {
        setFailure(result.error);
        return;
      }
      setFailure(null);
      await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
      command.onSuccess?.(result.value);
    },
  });

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !mutation.isPending) {
        setOpen(false);
      }
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [mutation.isPending, open]);

  const enter = (room: PrivateRoom): void => {
    setOpen(false);
    setView('rooms');
    onEnterRoom(room.catalogId, room.matrixRoomId);
  };

  const run = (command: RoomCommand): void => {
    setFailure(null);
    mutation.mutate(command);
  };

  const openHub = (): void => {
    setFailure(null);
    setView('rooms');
    setOpen(true);
  };

  return (
    <>
      <Button
        className="private-room-launcher"
        icon={<LockKeyhole aria-hidden="true" />}
        onClick={openHub}
        size="compact"
        tone="quiet"
      >
        {t('privateRooms.launcher')}
        {pendingInvitations === 0 ? null : (
          <span aria-label={t('privateRooms.pendingCount', { count: pendingInvitations })}>
            {pendingInvitations}
          </span>
        )}
      </Button>
      {createPortal(
        <AnimatePresence>
          {open ? (
            <motion.div
              animate={{ opacity: 1 }}
              className="private-room-overlay"
              exit={{ opacity: 0 }}
              initial={{ opacity: 0 }}
              key="private-room-overlay"
              onMouseDown={(event) => {
                if (event.target === event.currentTarget && !mutation.isPending) {
                  setOpen(false);
                }
              }}
              transition={{ duration: 0.14 }}
            >
              <motion.aside
                animate={{ x: 0 }}
                aria-labelledby="private-room-hub-title"
                aria-modal="true"
                className="private-room-sheet"
                exit={{ x: '100%' }}
                initial={{ x: '100%' }}
                role="dialog"
                transition={{ damping: 34, stiffness: 360, type: 'spring' }}
              >
                <div className="private-room-sheet__topbar">
                  <div>
                    <p className="eyebrow">{t('privateRooms.eyebrow')}</p>
                    <strong id="private-room-hub-title">{t('privateRooms.title')}</strong>
                  </div>
                  <button
                    aria-label={t('privateRooms.action.close')}
                    className="private-room-close"
                    disabled={mutation.isPending}
                    onClick={() => setOpen(false)}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </div>

                {view === 'create' ? (
                  <PrivateRoomCreateFlow
                    failure={failure}
                    onCancel={() => {
                      setFailure(null);
                      setView('rooms');
                    }}
                    onCreate={(input: CreatePrivateRoomInput) =>
                      run({
                        execute: async () =>
                          await coordinator.createAndJoin(identifiers.next(), input),
                        onSuccess: enter,
                      })
                    }
                    pending={mutation.isPending}
                  />
                ) : view === 'governance' && currentRoom !== null ? (
                  <PrivateRoomGovernance
                    coordinator={coordinator}
                    onExitRoom={() => {
                      setOpen(false);
                      onExitRoom();
                    }}
                    principalId={principal.principalId}
                    recentlyAuthenticated={principal.recentlyAuthenticated}
                    room={currentRoom}
                    rooms={privateRooms}
                  />
                ) : (
                  <RoomDirectory
                    currentCatalogId={currentCatalogId}
                    failure={failure ?? (list.data?.ok === false ? list.data.error : null)}
                    loading={list.isPending}
                    onAccept={(room) =>
                      run({ execute: async () => await coordinator.accept(room), onSuccess: enter })
                    }
                    onCreate={() => {
                      setFailure(null);
                      setView('create');
                    }}
                    onDecline={(room) =>
                      run({
                        execute: async () => await coordinator.decline(room),
                      })
                    }
                    onManage={currentRoom === null ? undefined : () => setView('governance')}
                    onOpen={(room) =>
                      run({ execute: async () => await coordinator.open(room), onSuccess: enter })
                    }
                    pending={mutation.isPending}
                    principalId={principal.principalId}
                    rooms={rooms}
                  />
                )}
              </motion.aside>
            </motion.div>
          ) : null}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}

type RoomDirectoryProps = {
  readonly currentCatalogId: string;
  readonly failure: PrivateRoomFailure | null;
  readonly loading: boolean;
  readonly onAccept: (room: PrivateRoom) => void;
  readonly onCreate: () => void;
  readonly onDecline: (room: PrivateRoom) => void;
  readonly onManage: (() => void) | undefined;
  readonly onOpen: (room: PrivateRoom) => void;
  readonly pending: boolean;
  readonly principalId: string;
  readonly rooms: readonly PrivateRoom[];
};

function RoomDirectory({
  currentCatalogId,
  failure,
  loading,
  onAccept,
  onCreate,
  onDecline,
  onManage,
  onOpen,
  pending,
  principalId,
  rooms,
}: RoomDirectoryProps) {
  const { t } = useTranslation();
  const activeRooms = rooms.filter((room) => room.status === 'active');
  return (
    <section className="private-room-directory">
      <header>
        <div>
          <h2>{t('privateRooms.directory.title')}</h2>
          <p>{t('privateRooms.directory.detail')}</p>
        </div>
        <Button
          icon={<DoorOpen aria-hidden="true" />}
          onClick={onCreate}
          size="compact"
          tone="primary"
        >
          {t('privateRooms.action.create')}
        </Button>
      </header>

      {loading ? (
        <div className="private-room-directory__state" role="status">
          <LoaderCircle aria-hidden="true" className="private-room-spin" />
          <strong>{t('privateRooms.directory.loading')}</strong>
        </div>
      ) : failure !== null ? (
        <PrivateRoomFailureNotice failure={failure} />
      ) : activeRooms.length === 0 ? (
        <div className="private-room-directory__state">
          <LockKeyhole aria-hidden="true" />
          <strong>{t('privateRooms.directory.empty')}</strong>
          <p>{t('privateRooms.directory.emptyDetail')}</p>
        </div>
      ) : (
        <ol className="private-room-directory__list">
          {activeRooms.map((room) => {
            const membership = room.members.find((member) => member.principalId === principalId);
            const invited = membership?.status === 'invited';
            const current = room.catalogId === currentCatalogId;
            return (
              <li
                className={current ? 'private-room-card--current' : undefined}
                key={room.catalogId}
              >
                <div className="private-room-card__signal">
                  {invited ? (
                    <UserRoundCheck aria-hidden="true" />
                  ) : (
                    <LockKeyhole aria-hidden="true" />
                  )}
                </div>
                <div className="private-room-card__identity">
                  <span>
                    {invited
                      ? t('privateRooms.directory.invited')
                      : t('privateRooms.directory.joined')}
                  </span>
                  <strong>{room.name}</strong>
                  <p>{room.description || t('privateRooms.directory.noDescription')}</p>
                  <small>{room.catalogId}</small>
                </div>
                <div className="private-room-card__actions">
                  {invited ? (
                    <>
                      <Button
                        disabled={pending}
                        onClick={() => onDecline(room)}
                        size="compact"
                        tone="quiet"
                      >
                        {t('privateRooms.action.decline')}
                      </Button>
                      <Button
                        disabled={pending}
                        icon={<Check aria-hidden="true" />}
                        onClick={() => onAccept(room)}
                        size="compact"
                        tone="primary"
                      >
                        {t('privateRooms.action.accept')}
                      </Button>
                    </>
                  ) : current && onManage !== undefined ? (
                    <Button
                      disabled={pending}
                      icon={<Settings2 aria-hidden="true" />}
                      onClick={onManage}
                      size="compact"
                      tone="ghost"
                    >
                      {t('privateRooms.action.manage')}
                    </Button>
                  ) : (
                    <Button
                      disabled={pending}
                      icon={<ArrowRight aria-hidden="true" />}
                      onClick={() => onOpen(room)}
                      size="compact"
                      tone="ghost"
                    >
                      {t('privateRooms.action.open')}
                    </Button>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
      )}
      <footer>
        <span>{t('privateRooms.directory.security')}</span>
        <strong>{t('privateRooms.directory.total', { count: activeRooms.length })}</strong>
      </footer>
    </section>
  );
}
