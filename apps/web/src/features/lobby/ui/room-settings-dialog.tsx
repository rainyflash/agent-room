import { Banner, Button, Dialog, Segmented, Spinner } from '@agent-room/ui-system';
import { Bot, DoorOpen, Gavel, Globe2, KeyRound, List, Pin, UsersRound } from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { AutomationSettings } from '@/features/automation/ui/automation-settings';
import { useModerationCapabilities } from '@/features/moderation/data/moderation-queries';
import { ModerationSettings } from '@/features/moderation/ui/moderation-settings';
import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  usePrivateRoomListWith,
  usePrivateRoomOrigin,
  usePrivateRoomPeople,
} from '@/features/private-rooms/data/private-room-queries';
import { allows, memberFor, type PrivateRoom } from '@/features/private-rooms/domain/private-room';
import {
  managesSubRoom,
  type SubRoom,
  type SubRoomList,
} from '@/features/private-rooms/domain/sub-rooms';
import { PrivateRoomAgentAccess } from '@/features/private-rooms/ui/private-room-agent-access';
import { PrivateRoomMembers } from '@/features/private-rooms/ui/private-room-members';
import { PrivateRoomPublicSettings } from '@/features/private-rooms/ui/private-room-public-settings';
import type { LobbyMemberName } from '@/features/lobby/domain/lobby';
import type { RoomNotice } from '@/features/room-notice/domain/room-notice';
import { RoomNoticeEditor } from '@/features/room-notice/ui/room-notice-editor';
import type { WebSession } from '@/features/session/domain/session';
import { RoomSubRoomsSettings } from '@/features/sub-rooms/ui/room-sub-rooms-settings';
import { SubRoomSettings } from '@/features/sub-rooms/ui/sub-room-settings';

import { agentCountsByOwner } from '../domain/agent-owners';
import { useAgentOwners } from './agent-owners';
import './room-settings.css';

export type RoomSettingsSection =
  | 'agent-code'
  | 'automation'
  | 'listing'
  | 'members'
  | 'moderation'
  | 'notice'
  | 'sub-room'
  | 'sub-rooms';

/** 不替谁去问主人：只读房间页问到的那一份。 */
const noAgents: readonly string[] = [];

const sectionIcons: Readonly<Record<RoomSettingsSection, ReactNode>> = {
  'agent-code': <KeyRound aria-hidden="true" />,
  automation: <Bot aria-hidden="true" />,
  listing: <Globe2 aria-hidden="true" />,
  members: <UsersRound aria-hidden="true" />,
  moderation: <Gavel aria-hidden="true" />,
  notice: <Pin aria-hidden="true" />,
  'sub-room': <DoorOpen aria-hidden="true" />,
  'sub-rooms': <List aria-hidden="true" />,
};

/** 眼前是子房间时：它在名单里的那一行，和它所在的整间房的名单。 */
export type RoomSettingsSubRoom = {
  readonly list: SubRoomList;
  readonly room: SubRoom;
};

export type RoomSettingsDialogProps = {
  readonly catalogId: string;
  readonly roomName: string;
  readonly principal: WebSession;
  /** 房间状态里此刻的须知，“须知”一节拿它当底稿。 */
  readonly notice?: RoomNotice | undefined;
  /** 房间里的人的昵称，“须知”一节写上次谁改的。 */
  readonly memberNames?: readonly LobbyMemberName[] | undefined;
  /** 打开时先看哪一节；不给就是第一节。 */
  readonly initialSection?: RoomSettingsSection | undefined;
  readonly onClose: () => void;
  /** 退出或归档了这个私人房间。 */
  readonly onExitRoom: () => void;
  /**
   * 眼前是子房间（`specs/redesign/sub-rooms.md`“网页”）：管得了的人第一节是“这间子房间”；成员、公开、
   * Agent 进门这几节是整间房的（邀请链接、口令、公开），不出现。
   */
  readonly subRoom?: RoomSettingsSubRoom | null | undefined;
  /**
   * 从左栏那一行的“⋯”打开的别的子房间：只有“这间子房间”一节。公告、自动发言、治理要这一页的房间状态，
   * 不是眼前这一间时拿不到。
   */
  readonly subRoomOnly?: boolean | undefined;
};

/**
 * 房间设置：一个对话框，分节切换。私人房间有“成员”和“公开”（管理者还有“公告”和“Agent 进门”），每个房间都有
 * “自动发言”，房间管理者还有“治理”。子房间没有成员、公开、Agent 进门，管得了它的人多一节“这间子房间”。
 * 几节同时挂着、只显示一节：切过去再切回来，填到一半的内容和刚生成的口令都还在。
 */
export function RoomSettingsDialog({
  catalogId,
  roomName,
  principal,
  notice,
  memberNames,
  initialSection,
  onClose,
  onExitRoom,
  subRoom = null,
  subRoomOnly = false,
}: RoomSettingsDialogProps) {
  const { t } = useTranslation();
  const { privateRoomListings, privateRooms, moderation, roomNotices, subRooms } = useAppServices();
  // 子房间不在房间列表里，单独问这一间（`usePrivateRoomListWith`）。
  const list = usePrivateRoomListWith(
    privateRooms,
    principal.principalId,
    catalogId,
    true,
    subRoom !== null,
  );
  const capabilities = useModerationCapabilities(moderation, catalogId);
  const room = list.data?.ok === true ? joinedRoom(list.data.value, catalogId, principal) : null;
  // 子房间的父房间：“管这一间的”认谁是父房间的管理员。你在父房间里，它就在房间列表里。
  const parentRoom =
    subRoom === null || list.data?.ok !== true
      ? null
      : (list.data.value.find((entry) => entry.catalogId === subRoom.list.parent.catalogId) ??
        null);
  const origin = usePrivateRoomOrigin(privateRooms, catalogId, room !== null);
  const serverReadable =
    room !== null && origin.data?.ok === true && origin.data.value.ownerIsNetworkAgent;
  const canManageRoom =
    room !== null &&
    (room.ownerPrincipalId === principal.principalId ||
      allows(memberFor(room, principal.principalId), 'manage'));
  const canModerate =
    capabilities.data?.ok === true &&
    (capabilities.data.value.canModerateRoom || capabilities.data.value.canReadAudit);
  const wholeRoom = room !== null && subRoom === null;
  const sections: readonly RoomSettingsSection[] = subRoomOnly
    ? ['sub-room']
    : [
        ...(subRoom !== null && managesSubRoom(subRoom.room) ? (['sub-room'] as const) : []),
        ...(wholeRoom ? (['members'] as const) : []),
        ...(wholeRoom && privateRoomListings !== undefined ? (['listing'] as const) : []),
        ...(wholeRoom && canManageRoom && subRooms !== undefined ? (['sub-rooms'] as const) : []),
        ...(canManageRoom && roomNotices !== undefined ? (['notice'] as const) : []),
        ...(canManageRoom && subRoom === null ? (['agent-code'] as const) : []),
        'automation',
        ...(canModerate ? (['moderation'] as const) : []),
      ];
  const [chosen, setChosen] = useState<RoomSettingsSection | undefined>(initialSection);
  const current =
    chosen !== undefined && sections.includes(chosen) ? chosen : (sections[0] ?? 'automation');
  const loading = list.isPending || capabilities.isPending;
  const privateFailed = !list.isPending && (list.isError || list.data?.ok !== true);

  return (
    <Dialog
      className="room-settings"
      closeLabel={t('roomSettings.close')}
      description={subRoom === null ? roomName : `${roomName} · ${subRoom.list.parent.name}`}
      onClose={onClose}
      size="wide"
      title={t(subRoom === null ? 'roomSettings.title' : 'subRooms.settings.title')}
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
          {serverReadable ? (
            <Banner
              role={null}
              title={t('privateRooms.origin.serverReadable.title')}
              tone="warning"
            >
              {t('privateRooms.origin.serverReadable.detail')}
            </Banner>
          ) : null}
          {sections.length > 1 ? (
            <Segmented
              // 七节放一行时图标挤掉了名字：只留名字。
              {...(sections.length >= 7 ? { className: 'room-settings__tabs--dense' } : {})}
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
                canManageRoom={canManageRoom}
                catalogId={catalogId}
                memberNames={memberNames}
                notice={notice}
                onClose={onClose}
                onExitRoom={onExitRoom}
                parentRoom={parentRoom}
                principal={principal}
                room={room}
                roomName={roomName}
                section={section}
                subRoom={subRoom}
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
  canManageRoom,
  catalogId,
  roomName,
  principal,
  room,
  notice,
  memberNames,
  onClose,
  onExitRoom,
  parentRoom,
  subRoom,
}: {
  readonly section: RoomSettingsSection;
  readonly canManageRoom: boolean;
  readonly catalogId: string;
  readonly roomName: string;
  readonly notice: RoomNotice | undefined;
  readonly memberNames: readonly LobbyMemberName[] | undefined;
  readonly principal: WebSession;
  readonly room: PrivateRoom | null;
  readonly onClose: () => void;
  readonly onExitRoom: () => void;
  readonly parentRoom: PrivateRoom | null;
  readonly subRoom: RoomSettingsSubRoom | null;
}) {
  const {
    accessManagement,
    automation,
    controlPlane,
    moderation,
    privateRoomListings,
    privateRoomMatrix,
    privateRooms,
    publicRoomJoins,
    roomNotices,
    roomRoleChanges,
  } = useAppServices();
  const coordinator = useMemo(
    () => new PrivateRoomCoordinator(privateRooms, privateRoomMatrix),
    [privateRoomMatrix, privateRooms],
  );
  const reauthenticate = (): void => {
    void controlPlane.beginAuthentication(
      `${window.location.pathname}${window.location.search}${window.location.hash}`,
      'reauthenticate',
    );
  };
  // 这间房里的 Agent 是谁的，房间页已经问过了（右栏“人”那一栏也数它）：这里只读问到的。
  const owners = useAgentOwners(noAgents);
  const agentCounts = useMemo(() => agentCountsByOwner(owners.values()), [owners]);
  switch (section) {
    case 'sub-room':
      return subRoom === null ? null : (
        <SubRoomSettings
          list={subRoom.list}
          managing={
            room === null
              ? null
              : {
                  members: room,
                  onReauthenticate: reauthenticate,
                  parent: parentRoom,
                  principalId: principal.principalId,
                  recentlyAuthenticated: principal.recentlyAuthenticated,
                }
          }
          onArchived={onClose}
          room={subRoom.room}
        />
      );
    case 'sub-rooms':
      return <RoomSubRoomsSettings parentId={catalogId} />;
    case 'members':
      return room === null ? null : (
        <PrivateRoomMembers
          agentCountOf={
            owners.size === 0 ? undefined : (principalId) => agentCounts.get(principalId) ?? 0
          }
          coordinator={coordinator}
          joins={publicRoomJoins}
          listings={privateRoomListings}
          onExitRoom={() => {
            onClose();
            onExitRoom();
          }}
          onReauthenticate={reauthenticate}
          principalId={principal.principalId}
          recentlyAuthenticated={principal.recentlyAuthenticated}
          roleChanges={roomRoleChanges}
          room={room}
          rooms={privateRooms}
        />
      );
    case 'listing':
      return room === null || privateRoomListings === undefined ? null : (
        <PrivateRoomPublicSettings
          listings={privateRoomListings}
          principalId={principal.principalId}
          room={room}
        />
      );
    case 'notice':
      return roomNotices === undefined ? null : (
        <RoomNoticeEditor
          catalogId={catalogId}
          gateway={roomNotices}
          memberNames={memberNames}
          notice={notice}
          selfMatrixUserId={principal.matrixUserId}
        />
      );
    case 'agent-code':
      return room === null ? null : (
        <PrivateRoomAgentAccess listings={privateRoomListings} room={room} rooms={privateRooms} />
      );
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
      return canManageRoom && room !== null ? (
        <ManagedRoomModeration
          catalogId={catalogId}
          onReauthenticate={reauthenticate}
          principal={principal}
          roomName={roomName}
        />
      ) : (
        <ModerationSettings
          catalogId={catalogId}
          gateway={moderation}
          onReauthenticate={reauthenticate}
          recentlyAuthenticated={principal.recentlyAuthenticated}
        />
      );
  }
}

/** 私人房间的房主和管理员：治理里多一块“交给 Agent 管”，给授权的人按成员名单写昵称。 */
function ManagedRoomModeration({
  catalogId,
  onReauthenticate,
  principal,
  roomName,
}: {
  readonly catalogId: string;
  readonly onReauthenticate: () => void;
  readonly principal: WebSession;
  readonly roomName: string;
}) {
  const { accessManagement, agentModeration, moderation, privateRooms } = useAppServices();
  const people = usePrivateRoomPeople(privateRooms, catalogId);
  const names = new Map(
    people.data?.ok === true
      ? people.data.value.map((person) => [person.principalId, person.displayName.trim()])
      : [],
  );
  return (
    <ModerationSettings
      agents={
        agentModeration === undefined
          ? undefined
          : {
              accessManagement,
              gateway: agentModeration,
              nameOf: (principalId) => {
                const name = names.get(principalId);
                return name === undefined || name === '' ? undefined : name;
              },
              principalId: principal.principalId,
              roomName,
            }
      }
      catalogId={catalogId}
      gateway={moderation}
      onReauthenticate={onReauthenticate}
      recentlyAuthenticated={principal.recentlyAuthenticated}
    />
  );
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
