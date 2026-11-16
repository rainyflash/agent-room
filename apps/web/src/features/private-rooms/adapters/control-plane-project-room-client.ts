import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  repositoryClaimSchema,
  roomRepositoryBindingResponseSchema,
  roomRepositoryCheckSchema,
  type ClaimRepositoryInput,
  type ProjectRoomGateway,
  type RepositoryClaim,
  type RoomRepositoryBinding,
  type RoomRepositoryCheck,
} from '@/features/private-rooms/domain/project-rooms';
import type { Result } from '@/shared/result';

import { PrivateRoomHttp, type PrivateRoomHttpOptions } from './private-room-http';

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

/**
 * 项目房间的认领和绑定（`specs/redesign/project-rooms.md`“接口”，3P-2b）。和私人房间的客户端一样经组合根
 * 注入的 fetch 带着登录发请求；回答按新的 schema 严格校验。
 */
export class ControlPlaneProjectRoomClient implements ProjectRoomGateway {
  readonly #http: PrivateRoomHttp;

  constructor(options: PrivateRoomHttpOptions) {
    this.#http = new PrivateRoomHttp(options);
  }

  claim(input: ClaimRepositoryInput): Answer<RepositoryClaim> {
    return this.#http.request(
      '/project-rooms/claims',
      {
        body: JSON.stringify({
          repository: input.repository,
          ...(input.catalogId === undefined ? {} : { catalogId: input.catalogId }),
        }),
        method: 'POST',
      },
      repositoryClaimSchema,
    );
  }

  inspectClaim(claimId: string): Answer<RepositoryClaim> {
    return this.#http.request(claimPath(claimId), { method: 'GET' }, repositoryClaimSchema);
  }

  checkClaim(claimId: string): Answer<RepositoryClaim> {
    return this.#http.request(
      `${claimPath(claimId)}/check`,
      { method: 'POST' },
      repositoryClaimSchema,
    );
  }

  cancelClaim(claimId: string): Answer<void> {
    return this.#http.send(claimPath(claimId), { method: 'DELETE' });
  }

  binding(catalogId: string): Answer<RoomRepositoryBinding | null> {
    return this.#http.request(
      repositoryPath(catalogId),
      { method: 'GET' },
      roomRepositoryBindingResponseSchema,
    );
  }

  unbind(catalogId: string): Answer<void> {
    return this.#http.send(repositoryPath(catalogId), { method: 'DELETE' });
  }

  checkBinding(catalogId: string): Answer<RoomRepositoryCheck> {
    return this.#http.request(
      `${repositoryPath(catalogId)}/check`,
      { method: 'POST' },
      roomRepositoryCheckSchema,
    );
  }
}

function claimPath(claimId: string): string {
  return `/project-rooms/claims/${encodeURIComponent(claimId)}`;
}

function repositoryPath(catalogId: string): string {
  return `/private-rooms/${encodeURIComponent(catalogId)}/repository`;
}
