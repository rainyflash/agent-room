import {
  privateRoomSchema,
  type PrivateRoom,
  type PrivateRoomFailure,
} from '@/features/private-rooms/domain/private-room';
import {
  answeredJoinRequestSchema,
  pendingJoinRequestsSchema,
  type JoinRequestDecision,
  type PendingJoinRequest,
} from '@/features/private-rooms/domain/public-room-join';
import {
  subRoomCategoriesSchema,
  subRoomJoinStateSchema,
  subRoomListSchema,
  subRoomPolicySchema,
  type CreateSubRoomInput,
  type ReviseSubRoomCategoryInput,
  type ReviseSubRoomInput,
  type SubRoomCategory,
  type SubRoomGateway,
  type SubRoomJoinState,
  type SubRoomList,
  type SubRoomPolicy,
  type SubRoomPolicySettings,
  type SubRoomRef,
} from '@/features/private-rooms/domain/sub-rooms';
import { ok, type Result } from '@/shared/result';

import { PrivateRoomHttp, type PrivateRoomHttpOptions } from './private-room-http';

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

/** 子房间的接口（`specs/redesign/sub-rooms.md`“接口”），都挂在父房间下面。 */
export class ControlPlaneSubRoomClient implements SubRoomGateway {
  readonly #http: PrivateRoomHttp;

  constructor(options: PrivateRoomHttpOptions) {
    this.#http = new PrivateRoomHttp(options);
  }

  subRooms(roomId: string, options: { readonly archived?: boolean } = {}): Answer<SubRoomList> {
    const query = options.archived === true ? '?archived=1' : '';
    return this.#http.request(
      `${roomPath(roomId)}/sub-rooms${query}`,
      { method: 'GET' },
      subRoomListSchema,
    );
  }

  create(parentId: string, input: CreateSubRoomInput): Answer<PrivateRoom> {
    const { catalogId, ...body } = input;
    return this.#http.request(
      `${roomPath(parentId)}/sub-rooms`,
      {
        method: 'POST',
        headers: { 'Idempotency-Key': catalogId },
        body: JSON.stringify(body),
      },
      privateRoomSchema,
    );
  }

  revise(room: SubRoomRef, input: ReviseSubRoomInput): Answer<PrivateRoom> {
    return this.#http.request(
      subRoomPath(room),
      { method: 'PUT', body: JSON.stringify(input) },
      privateRoomSchema,
    );
  }

  archive(room: SubRoomRef): Answer<void> {
    return this.#http.send(subRoomPath(room), { method: 'DELETE' });
  }

  restore(room: SubRoomRef): Answer<void> {
    return this.#http.send(`${subRoomPath(room)}/restore`, { method: 'POST' });
  }

  join(room: SubRoomRef): Answer<PrivateRoom> {
    return this.#http.request(
      `${subRoomPath(room)}/membership`,
      { method: 'POST' },
      privateRoomSchema,
    );
  }

  leave(room: SubRoomRef): Answer<void> {
    return this.#http.send(`${subRoomPath(room)}/membership`, { method: 'DELETE' });
  }

  requestToJoin(room: SubRoomRef, message: string): Answer<SubRoomJoinState> {
    return this.#http.request(
      `${subRoomPath(room)}/join-requests`,
      { method: 'POST', body: JSON.stringify({ message }) },
      subRoomJoinStateSchema,
    );
  }

  withdrawJoinRequest(room: SubRoomRef): Answer<void> {
    return this.#http.send(`${subRoomPath(room)}/join-requests`, { method: 'DELETE' });
  }

  async joinRequests(room: SubRoomRef): Answer<readonly PendingJoinRequest[]> {
    const response = await this.#http.request(
      `${subRoomPath(room)}/join-requests`,
      { method: 'GET' },
      pendingJoinRequestsSchema,
    );
    return response.ok ? ok(response.value.requests) : response;
  }

  async answerJoinRequest(
    room: SubRoomRef,
    principalId: string,
    decision: JoinRequestDecision,
  ): Answer<void> {
    const response = await this.#http.request(
      `${subRoomPath(room)}/join-requests/${encodeURIComponent(principalId)}`,
      { method: 'PUT', body: JSON.stringify({ decision }) },
      answeredJoinRequestSchema,
    );
    return response.ok ? ok(undefined) : response;
  }

  admit(room: SubRoomRef, principalId: string): Answer<void> {
    return this.#http.send(memberPath(room, principalId), { method: 'PUT' });
  }

  remove(room: SubRoomRef, principalId: string): Answer<void> {
    return this.#http.send(memberPath(room, principalId), { method: 'DELETE' });
  }

  ban(room: SubRoomRef, principalId: string): Answer<void> {
    return this.#http.send(`${memberPath(room, principalId)}/ban`, { method: 'POST' });
  }

  unban(room: SubRoomRef, principalId: string): Answer<void> {
    return this.#http.send(`${memberPath(room, principalId)}/unban`, { method: 'POST' });
  }

  grantManager(room: SubRoomRef, principalId: string): Answer<void> {
    return this.#http.send(managerPath(room, principalId), { method: 'PUT' });
  }

  revokeManager(room: SubRoomRef, principalId: string): Answer<void> {
    return this.#http.send(managerPath(room, principalId), { method: 'DELETE' });
  }

  createCategory(parentId: string, name: string): Answer<readonly SubRoomCategory[]> {
    return this.#categories(`${roomPath(parentId)}/sub-room-categories`, {
      method: 'POST',
      body: JSON.stringify({ name }),
    });
  }

  reviseCategory(
    parentId: string,
    categoryId: string,
    input: ReviseSubRoomCategoryInput,
  ): Answer<readonly SubRoomCategory[]> {
    return this.#categories(categoryPath(parentId, categoryId), {
      method: 'PUT',
      body: JSON.stringify(input),
    });
  }

  deleteCategory(parentId: string, categoryId: string): Answer<readonly SubRoomCategory[]> {
    return this.#categories(categoryPath(parentId, categoryId), { method: 'DELETE' });
  }

  policy(parentId: string): Answer<SubRoomPolicy> {
    return this.#http.request(
      `${roomPath(parentId)}/sub-room-policy`,
      { method: 'GET' },
      subRoomPolicySchema,
    );
  }

  changePolicy(parentId: string, settings: SubRoomPolicySettings): Answer<SubRoomPolicy> {
    return this.#http.request(
      `${roomPath(parentId)}/sub-room-policy`,
      { method: 'PUT', body: JSON.stringify(settings) },
      subRoomPolicySchema,
    );
  }

  async #categories(path: string, init: RequestInit): Answer<readonly SubRoomCategory[]> {
    const response = await this.#http.request(path, init, subRoomCategoriesSchema);
    return response.ok ? ok(response.value.categories) : response;
  }
}

function roomPath(catalogId: string): string {
  return `/private-rooms/${encodeURIComponent(catalogId)}`;
}

function subRoomPath({ parentId, subRoomId }: SubRoomRef): string {
  return `${roomPath(parentId)}/sub-rooms/${encodeURIComponent(subRoomId)}`;
}

function memberPath(room: SubRoomRef, principalId: string): string {
  return `${subRoomPath(room)}/members/${encodeURIComponent(principalId)}`;
}

function managerPath(room: SubRoomRef, principalId: string): string {
  return `${subRoomPath(room)}/managers/${encodeURIComponent(principalId)}`;
}

function categoryPath(parentId: string, categoryId: string): string {
  return `${roomPath(parentId)}/sub-room-categories/${encodeURIComponent(categoryId)}`;
}
