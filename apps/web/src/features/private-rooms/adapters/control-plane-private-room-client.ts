import {
  admittedAgentSchema,
  joinCodeSchema,
  privateRoomAgentAccessSchema,
  privateRoomAgentKnocksSchema,
  privateRoomListSchema,
  privateRoomOriginSchema,
  privateRoomPeopleSchema,
  privateRoomSchema,
  type CreatePrivateRoomInput,
  type JoinCode,
  type PrivateRoom,
  type PrivateRoomAgentAccess,
  type PrivateRoomAgentKnock,
  type PrivateRoomAgentMember,
  type PrivateRoomFailure,
  type PrivateRoomGateway,
  type PrivateRoomInvitationInput,
  type PrivateRoomOrigin,
  type PrivateRoomPermissions,
  type PrivateRoomPerson,
  type TransferPrivateRoomOwnershipInput,
} from '@/features/private-rooms/domain/private-room';
import {
  privateRoomInviteLinkSchema,
  privateRoomInvitePreviewSchema,
  redeemedPrivateRoomInviteSchema,
  type PrivateRoomInviteGateway,
  type PrivateRoomInviteLink,
  type PrivateRoomInvitePreview,
  type RedeemedPrivateRoomInvite,
} from '@/features/private-rooms/domain/private-room-invite-link';
import {
  privateRoomListingSchema,
  type PrivateRoomListing,
  type PrivateRoomListingGateway,
  type PrivateRoomListingSettings,
} from '@/features/private-rooms/domain/private-room-listing';
import {
  answeredJoinRequestSchema,
  pendingJoinRequestsSchema,
  publicRoomMembershipSchema,
  type JoinRequestDecision,
  type PendingJoinRequest,
  type PublicRoomEntryGateway,
  type PublicRoomJoinGateway,
  type PublicRoomMembership,
} from '@/features/private-rooms/domain/public-room-join';
import {
  roomRoleChangesSchema,
  type RoomRoleChange,
  type RoomRoleChangeGateway,
} from '@/features/private-rooms/domain/room-role-changes';
import {
  writtenRoomNoticeSchema,
  type RoomNoticeGateway,
  type WrittenRoomNotice,
} from '@/features/room-notice/domain/room-notice';
import { ok, type Result } from '@/shared/result';

import { PrivateRoomHttp, type PrivateRoomHttpOptions } from './private-room-http';

/** 放行时服务器替 Agent 进房间：第一次进加密房间要开加密存储、建身份，比平常的请求慢。 */
const ADMIT_TIMEOUT_MS = 45_000;

export type ControlPlanePrivateRoomClientOptions = PrivateRoomHttpOptions;

export class ControlPlanePrivateRoomClient
  implements
    PrivateRoomGateway,
    PrivateRoomInviteGateway,
    PrivateRoomListingGateway,
    PublicRoomEntryGateway,
    PublicRoomJoinGateway,
    RoomNoticeGateway,
    RoomRoleChangeGateway
{
  readonly #http: PrivateRoomHttp;

  constructor(options: ControlPlanePrivateRoomClientOptions) {
    this.#http = new PrivateRoomHttp(options);
  }

  async list(): Promise<Result<readonly PrivateRoom[], PrivateRoomFailure>> {
    const response = await this.#http.request(
      '/private-rooms',
      { method: 'GET' },
      privateRoomListSchema,
    );
    return response.ok ? ok(response.value.rooms) : response;
  }

  inspect(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}`, { method: 'GET' });
  }

  origin(catalogId: string): Promise<Result<PrivateRoomOrigin, PrivateRoomFailure>> {
    return this.#http.request(
      `/private-rooms/${encodeURIComponent(catalogId)}/origin`,
      { method: 'GET' },
      privateRoomOriginSchema,
    );
  }

  async people(
    catalogId: string,
  ): Promise<Result<readonly PrivateRoomPerson[], PrivateRoomFailure>> {
    const response = await this.#http.request(
      `/private-rooms/${encodeURIComponent(catalogId)}/people`,
      { method: 'GET' },
      privateRoomPeopleSchema,
    );
    return response.ok ? ok(response.value.people) : response;
  }

  async roleChanges(
    catalogId: string,
  ): Promise<Result<readonly RoomRoleChange[], PrivateRoomFailure>> {
    const response = await this.#http.request(
      `/private-rooms/${encodeURIComponent(catalogId)}/role-changes`,
      { method: 'GET' },
      roomRoleChangesSchema,
    );
    return response.ok ? ok(response.value.changes) : response;
  }

  create(
    catalogId: string,
    input: CreatePrivateRoomInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest('/private-rooms', {
      body: JSON.stringify(input),
      headers: { 'Idempotency-Key': catalogId },
      method: 'POST',
    });
  }

  invite(
    catalogId: string,
    invitation: PrivateRoomInvitationInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}/invitations`, {
      body: JSON.stringify({
        permissions: invitation.permissions,
        targetPrincipalId: invitation.principalId,
      }),
      method: 'POST',
    });
  }

  accept(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#membership(catalogId, 'accept');
  }

  decline(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#membership(catalogId, 'decline');
  }

  leave(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#membership(catalogId, 'leave');
  }

  remove(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(this.#memberPath(catalogId, principalId), { method: 'DELETE' });
  }

  ban(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`${this.#memberPath(catalogId, principalId)}/ban`, { method: 'POST' });
  }

  unban(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`${this.#memberPath(catalogId, principalId)}/unban`, {
      method: 'POST',
    });
  }

  updatePermissions(
    catalogId: string,
    principalId: string,
    permissions: PrivateRoomPermissions,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`${this.#memberPath(catalogId, principalId)}/permissions`, {
      body: JSON.stringify(permissions),
      method: 'PUT',
    });
  }

  transferOwnership(
    catalogId: string,
    input: TransferPrivateRoomOwnershipInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}/owner`, {
      body: JSON.stringify(input),
      method: 'PUT',
    });
  }

  rename(catalogId: string, name: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}/name`, {
      body: JSON.stringify({ name }),
      method: 'PUT',
    });
  }

  updateNotice(
    catalogId: string,
    text: string,
  ): Promise<Result<WrittenRoomNotice, PrivateRoomFailure>> {
    return this.#http.request(
      `/private-rooms/${encodeURIComponent(catalogId)}/notice`,
      { body: JSON.stringify({ text }), method: 'PUT' },
      writtenRoomNoticeSchema,
    );
  }

  archive(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(`/private-rooms/${encodeURIComponent(catalogId)}`, {
      method: 'DELETE',
    });
  }

  agentAccess(catalogId: string): Promise<Result<PrivateRoomAgentAccess, PrivateRoomFailure>> {
    return this.#http.request(
      this.#agentAccessPath(catalogId),
      { method: 'GET' },
      privateRoomAgentAccessSchema,
    );
  }

  async agentKnocks(
    catalogId: string,
  ): Promise<Result<readonly PrivateRoomAgentKnock[], PrivateRoomFailure>> {
    const response = await this.#http.request(
      `${this.#agentAccessPath(catalogId)}/knocks`,
      { method: 'GET' },
      privateRoomAgentKnocksSchema,
    );
    return response.ok ? ok(response.value.knocks) : response;
  }

  async admitKnock(
    catalogId: string,
    agentId: string,
  ): Promise<Result<PrivateRoomAgentMember, PrivateRoomFailure>> {
    const response = await this.#http.request(
      `${this.#agentAccessPath(catalogId)}/agents/${encodeURIComponent(agentId)}`,
      { method: 'PUT' },
      admittedAgentSchema,
      ADMIT_TIMEOUT_MS,
    );
    return response.ok ? ok(response.value.agent) : response;
  }

  declineKnock(catalogId: string, agentId: string): Promise<Result<void, PrivateRoomFailure>> {
    return this.#http.send(
      `${this.#agentAccessPath(catalogId)}/knocks/${encodeURIComponent(agentId)}`,
      {
        method: 'DELETE',
      },
    );
  }

  generateJoinCode(catalogId: string): Promise<Result<JoinCode, PrivateRoomFailure>> {
    return this.#http.request(
      `${this.#agentAccessPath(catalogId)}/code`,
      { method: 'PUT' },
      joinCodeSchema,
    );
  }

  revealJoinCode(catalogId: string): Promise<Result<JoinCode, PrivateRoomFailure>> {
    return this.#http.request(
      `${this.#agentAccessPath(catalogId)}/code`,
      { method: 'GET' },
      joinCodeSchema,
    );
  }

  disableJoinCode(catalogId: string): Promise<Result<void, PrivateRoomFailure>> {
    return this.#http.send(`${this.#agentAccessPath(catalogId)}/code`, { method: 'DELETE' });
  }

  inviteLink(catalogId: string): Promise<Result<PrivateRoomInviteLink, PrivateRoomFailure>> {
    return this.#http.request(
      this.#inviteLinkPath(catalogId),
      { method: 'GET' },
      privateRoomInviteLinkSchema,
    );
  }

  createInviteLink(
    catalogId: string,
    permissions?: PrivateRoomPermissions,
  ): Promise<Result<PrivateRoomInviteLink, PrivateRoomFailure>> {
    return this.#http.request(
      this.#inviteLinkPath(catalogId),
      {
        body: JSON.stringify(permissions === undefined ? {} : { permissions }),
        method: 'POST',
      },
      privateRoomInviteLinkSchema,
    );
  }

  disableInviteLink(catalogId: string): Promise<Result<void, PrivateRoomFailure>> {
    return this.#http.send(this.#inviteLinkPath(catalogId), { method: 'DELETE' });
  }

  // 秘密放在请求正文里，不进地址：地址会进服务器和代理的日志。
  previewInvite(secret: string): Promise<Result<PrivateRoomInvitePreview, PrivateRoomFailure>> {
    return this.#http.request(
      '/private-room-invites/preview',
      { body: JSON.stringify({ secret }), method: 'POST' },
      privateRoomInvitePreviewSchema,
    );
  }

  redeemInvite(secret: string): Promise<Result<RedeemedPrivateRoomInvite, PrivateRoomFailure>> {
    return this.#http.request(
      '/private-room-invites/redeem',
      { body: JSON.stringify({ secret }), method: 'POST' },
      redeemedPrivateRoomInviteSchema,
    );
  }

  listing(catalogId: string): Promise<Result<PrivateRoomListing, PrivateRoomFailure>> {
    return this.#http.request(
      this.#listingPath(catalogId),
      { method: 'GET' },
      privateRoomListingSchema,
    );
  }

  updateListing(
    catalogId: string,
    settings: PrivateRoomListingSettings,
  ): Promise<Result<PrivateRoomListing, PrivateRoomFailure>> {
    return this.#http.request(
      this.#listingPath(catalogId),
      {
        body: JSON.stringify({
          allowNetworkAgents: settings.allowNetworkAgents,
          joinMode: settings.joinMode,
          listed: settings.listed,
          memberPermissions: settings.memberPermissions,
        }),
        method: 'PUT',
      },
      privateRoomListingSchema,
    );
  }

  changeShareCode(catalogId: string): Promise<Result<PrivateRoomListing, PrivateRoomFailure>> {
    return this.#http.request(
      `${this.#listingPath(catalogId)}/share-code`,
      { method: 'POST' },
      privateRoomListingSchema,
    );
  }

  join(catalogId: string): Promise<Result<PublicRoomMembership, PrivateRoomFailure>> {
    return this.#http.request(
      `/private-rooms/${encodeURIComponent(catalogId)}/join`,
      { method: 'POST' },
      publicRoomMembershipSchema,
    );
  }

  requestToJoin(
    catalogId: string,
    message: string,
  ): Promise<Result<PublicRoomMembership, PrivateRoomFailure>> {
    return this.#http.request(
      this.#joinRequestsPath(catalogId),
      { body: JSON.stringify({ message }), method: 'POST' },
      publicRoomMembershipSchema,
    );
  }

  withdrawJoinRequest(catalogId: string): Promise<Result<void, PrivateRoomFailure>> {
    return this.#http.send(this.#joinRequestsPath(catalogId), { method: 'DELETE' });
  }

  async joinRequests(
    catalogId: string,
  ): Promise<Result<readonly PendingJoinRequest[], PrivateRoomFailure>> {
    const response = await this.#http.request(
      this.#joinRequestsPath(catalogId),
      { method: 'GET' },
      pendingJoinRequestsSchema,
    );
    return response.ok ? ok(response.value.requests) : response;
  }

  async answerJoinRequest(
    catalogId: string,
    principalId: string,
    decision: JoinRequestDecision,
  ): Promise<Result<void, PrivateRoomFailure>> {
    const response = await this.#http.request(
      `${this.#joinRequestsPath(catalogId)}/${encodeURIComponent(principalId)}`,
      { body: JSON.stringify({ decision }), method: 'PUT' },
      answeredJoinRequestSchema,
    );
    return response.ok ? ok(undefined) : response;
  }

  removeCodeAgent(catalogId: string, agentId: string): Promise<Result<void, PrivateRoomFailure>> {
    return this.#http.send(
      `${this.#agentAccessPath(catalogId)}/agents/${encodeURIComponent(agentId)}`,
      {
        method: 'DELETE',
      },
    );
  }

  #joinRequestsPath(catalogId: string): string {
    return `/private-rooms/${encodeURIComponent(catalogId)}/join-requests`;
  }

  #listingPath(catalogId: string): string {
    return `/private-rooms/${encodeURIComponent(catalogId)}/listing`;
  }

  #inviteLinkPath(catalogId: string): string {
    return `/private-rooms/${encodeURIComponent(catalogId)}/invite-link`;
  }

  #agentAccessPath(catalogId: string): string {
    return `/private-rooms/${encodeURIComponent(catalogId)}/agent-access`;
  }

  #membership(
    catalogId: string,
    action: 'accept' | 'decline' | 'leave',
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#roomRequest(
      `/private-rooms/${encodeURIComponent(catalogId)}/membership/${action}`,
      { method: 'POST' },
    );
  }

  #memberPath(catalogId: string, principalId: string): string {
    return `/private-rooms/${encodeURIComponent(catalogId)}/members/${encodeURIComponent(principalId)}`;
  }

  #roomRequest(path: string, init: RequestInit): Promise<Result<PrivateRoom, PrivateRoomFailure>> {
    return this.#http.request(path, init, privateRoomSchema);
  }
}
