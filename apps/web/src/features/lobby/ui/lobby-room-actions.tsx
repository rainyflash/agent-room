import { Button } from '@agent-room/ui-system';
import { ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { AutomationGrantHub } from '@/features/automation/ui/automation-grant-hub';
import { ModerationHub } from '@/features/moderation/ui/moderation-hub';
import { PrivateRoomHub } from '@/features/private-rooms/ui/private-room-hub';
import type { WebSession } from '@/features/session/domain/session';

export function LobbyRoomActions({
  catalogId,
  roomName,
  principal,
  onEnterRoom,
  onExitRoom,
  onOpenSecurity,
}: {
  readonly catalogId: string;
  readonly roomName: string;
  readonly principal: WebSession;
  readonly onEnterRoom: (catalogId: string, roomId: string) => void;
  readonly onExitRoom: () => void;
  readonly onOpenSecurity: () => void;
}) {
  const { t } = useTranslation();
  const { accessManagement, automation, controlPlane, moderation } = useAppServices();
  const reauthenticate = (): void => {
    void controlPlane.beginAuthentication(
      `${window.location.pathname}${window.location.search}${window.location.hash}`,
    );
  };
  return (
    <>
      <ModerationHub
        catalogId={catalogId}
        gateway={moderation}
        onReauthenticate={reauthenticate}
        recentlyAuthenticated={principal.recentlyAuthenticated}
        roomName={roomName}
      />
      <AutomationGrantHub
        accessManagement={accessManagement}
        automation={automation}
        catalogId={catalogId}
        onReauthenticate={reauthenticate}
        recentlyAuthenticated={principal.recentlyAuthenticated}
        roomName={roomName}
      />
      <Button
        aria-label={t('security.launcher')}
        icon={<ShieldCheck aria-hidden="true" />}
        onClick={onOpenSecurity}
        size="compact"
        tone="quiet"
      >
        {t('security.launcher')}
      </Button>
      <PrivateRoomHub
        currentCatalogId={catalogId}
        onEnterRoom={onEnterRoom}
        onExitRoom={onExitRoom}
        principal={principal}
      />
    </>
  );
}
