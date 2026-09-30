import { Banner, Button, Dialog, Segmented, Spinner } from '@agent-room/ui-system';
import { Bot, Gavel, KeyRound, Settings2, UsersRound } from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { AutomationSettings } from '@/features/automation/ui/automation-settings';
import { useModerationCapabilities } from '@/features/moderation/data/moderation-queries';
import { ModerationSettings } from '@/features/moderation/ui/moderation-settings';
import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import { usePrivateRoomList } from '@/features/private-rooms/data/private-room-queries';
import { allows, memberFor, type PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { PrivateRoomAgentAccess } from '@/features/private-rooms/ui/private-room-agent-access';
import { PrivateRoomMembers } from '@/features/private-rooms/ui/private-room-members';
import type { WebSession } from '@/features/session/domain/session';

import './room-settings.css';

export type RoomSettingsSection = 'agent-code' | 'automation' | 'members' | 'moderation';

const sectionIcons: Readonly<Record<RoomSettingsSection, ReactNode>> = {
  'agent-code': <KeyRound aria-hidden="true" />,
  automation: <Bot aria-hidden="true" />,
  members: <UsersRound aria-hidden="true" />,
  moderation: <Gavel aria-hidden="true" />,
};

export type RoomSettingsDialogProps = {
  readonly catalogId: string;
  readonly roomName: string;
  readonly principal: WebSession;
  /** 打开时先看哪一节；不给就是第一节。 */
  readonly initialSection?: RoomSettingsSection | undefined;
  readonly onClose: () => void;
  /** 退出或归档了这个私人房间。 */
  readonly onExitRoom: () => void;
};

/**
 * 房间设置：一个对话框，分节切换。私人房间有“成员”（管理者还有“Agent 口令”），每个房间都有
 * “自动发言”，房间管理者还有“治理”。几节同时挂着、只显示一节：切过去再切回来，填到一半的内容
 * 和刚生成的口令都还在。
 */
export function RoomSettingsDialog({
  catalogId,
  roomName,
  principal,
  initialSection,
  onClose,
  onExitRoom,
}: RoomSettingsDialogProps) {
  const { t } = useTranslation();
  const { privateRooms, moderation } = useAppServices();
  const list = usePrivateRoomList(privateRooms, principal.principalId);
  const capabilities = useModerationCapabilities(moderation, catalogId);
  const room = list.data?.ok === true ? joinedRoom(list.data.value, catalogId, principal) : null;
  const canManageRoom =
    room !== null &&
    (room.ownerPrincipalId === principal.principalId ||
      allows(memberFor(room, principal.principalId), 'manage'));
  const canModerate =
    capabilities.data?.ok === true &&
    (capabilities.data.value.canModerateRoom || capabilities.data.value.canReadAudit);
  const sections: readonly RoomSettingsSection[] = [
    ...(room === null ? [] : (['members'] as const)),
    ...(canManageRoom ? (['agent-code'] as const) : []),
    'automation',
    ...(canModerate ? (['moderation'] as const) : []),
  ];
  const [chosen, setChosen] = useState<RoomSettingsSection | undefined>(initialSection);
  const current =
    chosen !== undefined && sections.includes(chosen) ? chosen : (sections[0] ?? 'automation');
  const loading = list.isPending || capabilities.isPending;
  const privateFailed = !list.isPending && (list.isError || !list.data.ok);

  return (
    <Dialog
      className="room-settings"
      closeLabel={t('roomSettings.close')}
      description={roomName}
      icon={<Settings2 />}
      onClose={onClose}
      size="wide"
      title={t('roomSettings.title')}
    >
      {loading ? (
        <p className="room-settings__state" role="status">
          <Spinner />
          {t('roomSettings.loading')}
        </p>
      ) : (
        <>
          {privateFailed ? (
            <Banner
              action={
                <Button onClick={() => void list.refetch()} size="compact" tone="quiet">
                  {t('roomSettings.retry')}
                </Button>
              }
              tone="warning"
            >
              {t('roomSettings.privateFailed')}
            </Banner>
          ) : null}
          {sections.length > 1 ? (
            <Segmented
              label={t('roomSettings.sections')}
              onChange={setChosen}
              options={sections.map((section) => ({
                icon: sectionIcons[section],
                label: t(`roomSettings.section.${section}`),
                value: section,
              }))}
              value={current}
            />
          ) : null}
          {sections.map((section) => (
            <section
              aria-label={t(`roomSettings.section.${section}`)}
              className="room-settings__section"
              hidden={section !== current}
              key={section}
            >
              <RoomSettingsContent
                catalogId={catalogId}
                onClose={onClose}
                onExitRoom={onExitRoom}
                principal={principal}
                room={room}
                roomName={roomName}
                section={section}
              />
            </section>
          ))}
        </>
      )}
    </Dialog>
  );
}

function RoomSettingsContent({
  section,
  catalogId,
  roomName,
  principal,
  room,
  onClose,
  onExitRoom,
}: {
  readonly section: RoomSettingsSection;
  readonly catalogId: string;
  readonly roomName: string;
  readonly principal: WebSession;
  readonly room: PrivateRoom | null;
  readonly onClose: () => void;
  readonly onExitRoom: () => void;
}) {
  const {
    accessManagement,
    automation,
    controlPlane,
    moderation,
    privateRoomMatrix,
    privateRooms,
  } = useAppServices();
  const coordinator = useMemo(
    () => new PrivateRoomCoordinator(privateRooms, privateRoomMatrix),
    [privateRoomMatrix, privateRooms],
  );
  const reauthenticate = (): void => {
    void controlPlane.beginAuthentication(
      `${window.location.pathname}${window.location.search}${window.location.hash}`,
    );
  };
  switch (section) {
    case 'members':
      return room === null ? null : (
        <PrivateRoomMembers
          coordinator={coordinator}
          onExitRoom={() => {
            onClose();
            onExitRoom();
          }}
          principalId={principal.principalId}
          recentlyAuthenticated={principal.recentlyAuthenticated}
          room={room}
          rooms={privateRooms}
        />
      );
    case 'agent-code':
      return room === null ? null : <PrivateRoomAgentAccess room={room} rooms={privateRooms} />;
    case 'automation':
      return (
        <AutomationSettings
          accessManagement={accessManagement}
          automation={automation}
          catalogId={catalogId}
          principalId={principal.principalId}
          roomName={roomName}
        />
      );
    case 'moderation':
      return (
        <ModerationSettings
          catalogId={catalogId}
          gateway={moderation}
          onReauthenticate={reauthenticate}
          recentlyAuthenticated={principal.recentlyAuthenticated}
        />
      );
  }
}

/** 你已经加入、还在用的这间私人房间；公共大厅和没加入的都不算。 */
function joinedRoom(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principal: WebSession,
): PrivateRoom | null {
  return (
    rooms.find(
      (room) =>
        room.catalogId === catalogId &&
        room.status === 'active' &&
        memberFor(room, principal.principalId)?.status === 'joined',
    ) ?? null
  );
}
