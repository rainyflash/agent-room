import { useOverlayContainer } from '@/shared/ui/overlay-container';
import { Button } from '@agent-room/ui-system';
import { UsersRound, X } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import { usePrivateRoomList } from '@/features/private-rooms/data/private-room-queries';
import { PrivateRoomGovernance } from '@/features/private-rooms/ui/private-room-governance';
import type { WebSession } from '@/features/session/domain/session';
import { useAppServices } from '@/app/app-services';

export type PrivateRoomHubProps = {
  readonly currentCatalogId: string;
  readonly onExitRoom: () => void;
  readonly principal: WebSession;
};

/**
 * 当前私人房间的成员与权限。换房间、新建房间和别人的邀请都在“换个房间”“新建房间”里；
 * 公共大厅里没有这一项。
 */
export function PrivateRoomHub({ currentCatalogId, onExitRoom, principal }: PrivateRoomHubProps) {
  const { t } = useTranslation();
  const overlayContainer = useOverlayContainer();
  const { privateRoomMatrix, privateRooms } = useAppServices();
  const list = usePrivateRoomList(privateRooms, principal.principalId);
  const coordinator = useMemo(
    () => new PrivateRoomCoordinator(privateRooms, privateRoomMatrix),
    [privateRoomMatrix, privateRooms],
  );
  const [open, setOpen] = useState(false);
  const currentRoom =
    list.data?.ok === true
      ? (list.data.value.find(
          (room) =>
            room.catalogId === currentCatalogId &&
            room.status === 'active' &&
            room.members.some(
              (member) =>
                member.principalId === principal.principalId && member.status === 'joined',
            ),
        ) ?? null)
      : null;

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => {
      window.removeEventListener('keydown', closeOnEscape);
    };
  }, [open]);

  if (currentRoom === null) {
    return null;
  }

  return (
    <>
      <Button
        icon={<UsersRound aria-hidden="true" />}
        onClick={() => {
          setOpen(true);
        }}
        size="compact"
        tone="quiet"
      >
        {t('privateRooms.launcher')}
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
                if (event.target === event.currentTarget) {
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
                    <p className="eyebrow">{currentRoom.name}</p>
                    <strong id="private-room-hub-title">{t('privateRooms.title')}</strong>
                  </div>
                  <button
                    aria-label={t('privateRooms.action.close')}
                    className="private-room-close ar-icon-button"
                    onClick={() => {
                      setOpen(false);
                    }}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </div>
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
              </motion.aside>
            </motion.div>
          ) : null}
        </AnimatePresence>,
        overlayContainer,
      )}
    </>
  );
}
