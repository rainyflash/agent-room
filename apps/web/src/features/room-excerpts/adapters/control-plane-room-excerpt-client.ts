import {
  PrivateRoomHttp,
  type PrivateRoomHttpOptions,
} from '@/features/private-rooms/adapters/private-room-http';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  roomExcerptListSchema,
  sharedExcerptSchema,
  type ExcerptBounds,
  type RoomExcerpt,
  type RoomExcerptGateway,
  type SharedExcerpt,
} from '@/features/room-excerpts/domain/room-excerpts';
import { ok, type Result } from '@/shared/result';

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

/**
 * 分享一段对话的三个接口（`specs/redesign/project-rooms.md`“接口”，3P-7a-1）。和私人房间的客户端一样经组合根
 * 注入的 fetch 带着登录发请求；回答按新的 schema 严格校验。选的那几句不对时，为什么在
 * `PrivateRoomFailure.reason`；太多了时等多久在 `retryAfterSeconds`。
 */
export class ControlPlaneRoomExcerptClient implements RoomExcerptGateway {
  readonly #http: PrivateRoomHttp;

  constructor(options: PrivateRoomHttpOptions) {
    this.#http = new PrivateRoomHttp(options);
  }

  share(catalogId: string, bounds: ExcerptBounds): Answer<SharedExcerpt> {
    return this.#http.request(
      excerptsPath(catalogId),
      {
        // 正文只有这两项：控制面多一个字段就回 400。
        body: JSON.stringify({
          firstEventId: bounds.firstEventId,
          lastEventId: bounds.lastEventId,
        }),
        method: 'POST',
      },
      sharedExcerptSchema,
    );
  }

  async list(catalogId: string): Answer<readonly RoomExcerpt[]> {
    const result = await this.#http.request(
      excerptsPath(catalogId),
      { method: 'GET' },
      roomExcerptListSchema,
    );
    return result.ok ? ok(result.value.excerpts) : result;
  }

  remove(catalogId: string, code: string): Answer<void> {
    return this.#http.send(`${excerptsPath(catalogId)}/${encodeURIComponent(code)}`, {
      method: 'DELETE',
    });
  }
}

function excerptsPath(catalogId: string): string {
  return `/private-rooms/${encodeURIComponent(catalogId)}/excerpts`;
}
