import { Banner } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import type { PrivateRoomListingGateway } from '@/features/private-rooms/domain/private-room-listing';
import type { SubRoomAccess } from '@/features/private-rooms/domain/sub-rooms';
import { PrivateRoomAgentKnocks } from '@/features/private-rooms/ui/private-room-agent-knocks';
import { PrivateRoomNetworkInvite } from '@/features/private-rooms/ui/private-room-network-invite';

export type SubRoomForNetworkAgents = {
  readonly catalogId: string;
  readonly name: string;
  readonly parent: { readonly catalogId: string; readonly name: string };
  readonly access: SubRoomAccess;
  /** 看的人管不管得了这一间：管得了才在这里列敲这一间的门的 Agent。 */
  readonly manages: boolean;
};

type Props = {
  readonly agentName?: string | undefined;
  readonly guide: string;
  readonly room: SubRoomForNetworkAgents;
  readonly rooms: PrivateRoomGateway;
  readonly listings?: PrivateRoomListingGateway | undefined;
  readonly onCopied?: (() => void) | undefined;
  readonly principalId?: string | null | undefined;
};

/**
 * 在子房间里请网络 Agent（agent-room-ops#112）。子房间没有口令，只收已经在父房间里的 Agent
 * （`agents.md` 第 6、7 节）：所以先照父房间的规矩请它进父房间（口令、敲门、直接进都照父房间的设置和你在
 * 父房间里管不管得了），话的最后再叫它拿子房间的房间号进这一间。有门的这一间它是敲门，管得了的人在下面放行；
 * 只能请进来的，网络 Agent 拿房间号进不来，只说这一句。
 */
export function SubRoomNetworkInvite({
  agentName,
  guide,
  room,
  rooms,
  listings,
  onCopied,
  principalId,
}: Props) {
  const { t } = useTranslation();
  const names = { parent: room.parent.name, room: room.name, roomNumber: room.catalogId };
  if (room.access === 'invite_only') {
    return (
      <Banner role={null} tone="info">
        {t('agentInvite.network.subRoom.inviteOnly', names)}
      </Banner>
    );
  }
  const door = room.access === 'request';
  return (
    <>
      <p className="agent-invite__note">{t('agentInvite.network.subRoom.note', names)}</p>
      <PrivateRoomNetworkInvite
        agentName={agentName}
        catalogId={room.parent.catalogId}
        followUp={t(
          door
            ? 'agentInvite.network.subRoom.followUpDoor'
            : 'agentInvite.network.subRoom.followUp',
          names,
        )}
        guide={guide}
        listings={listings}
        onCopied={onCopied}
        principalId={principalId}
        roomName={room.parent.name}
        rooms={rooms}
      />
      {door ? (
        room.manages ? (
          <PrivateRoomAgentKnocks
            catalogId={room.catalogId}
            emptySpinner={false}
            emptyText={t('agentInvite.network.subRoom.knockWaiting', names)}
            rooms={rooms}
          />
        ) : (
          <p className="agent-invite__note">{t('agentInvite.network.subRoom.doorMember', names)}</p>
        )
      ) : null}
    </>
  );
}
