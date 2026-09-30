import { Button } from '@agent-room/ui-system';
import { ArrowLeftRight, Plus } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { NewRoomDialog } from '@/features/room-directory/ui/new-room-dialog';
import { RoomSwitcherDialog } from '@/features/room-directory/ui/room-switcher-dialog';
import { useMyPrivateRooms } from '@/features/room-directory/ui/use-rooms';

import './rooms.css';

export type RoomActionsProps = {
  readonly currentCatalogId?: string | undefined;
  /** “房间”页本身就是房间列表，那里只留“新建房间”。 */
  readonly showSwitch?: boolean;
};

/** “换个房间”和“新建房间”：所有入口都打开同样的两个对话框。 */
export function RoomActions(props: RoomActionsProps) {
  const services = useOptionalAppServices();
  if (services === null) return null;
  return <ConnectedRoomActions {...props} />;
}

function ConnectedRoomActions({ currentCatalogId, showSwitch = true }: RoomActionsProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState<'new' | 'switch' | null>(null);
  const { invitations } = useMyPrivateRooms();
  const close = (): void => {
    setOpen(null);
  };
  return (
    <div className="room-actions">
      {showSwitch ? (
        <Button
          icon={<ArrowLeftRight aria-hidden="true" />}
          onClick={() => {
            setOpen('switch');
          }}
          size="compact"
          tone="quiet"
        >
          {t('rooms.switch')}
          {invitations.length === 0 ? null : (
            <>
              <span aria-hidden="true" className="room-actions__count">
                {invitations.length}
              </span>
              <span className="sr-only">
                {t('rooms.switch.invitations', { count: invitations.length })}
              </span>
            </>
          )}
        </Button>
      ) : null}
      <Button
        icon={<Plus aria-hidden="true" />}
        onClick={() => {
          setOpen('new');
        }}
        size="compact"
      >
        {t('rooms.new')}
      </Button>
      {open === 'switch' ? (
        <RoomSwitcherDialog
          currentCatalogId={currentCatalogId}
          onClose={close}
          onCreate={() => {
            setOpen('new');
          }}
        />
      ) : null}
      {open === 'new' ? <NewRoomDialog onClose={close} /> : null}
    </div>
  );
}
