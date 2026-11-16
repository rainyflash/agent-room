import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  roomLiveViewSchema,
  type RoomLiveView,
  type RoomLiveViewGateway,
} from '@/features/private-rooms/domain/room-live-view';
import type { Result } from '@/shared/result';

import { PrivateRoomHttp, type PrivateRoomHttpOptions } from './private-room-http';

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

/**
 * 分享页放实况的开关（`specs/redesign/project-rooms.md`“接口”，3P-6a）。和私人房间的客户端一样经组合根注入的
 * fetch 带着登录发请求；回答按新的 schema 严格校验。开不了时差的那一样在 `PrivateRoomFailure.condition`。
 */
export class ControlPlaneRoomLiveViewClient implements RoomLiveViewGateway {
  readonly #http: PrivateRoomHttp;

  constructor(options: PrivateRoomHttpOptions) {
    this.#http = new PrivateRoomHttp(options);
  }

  liveView(catalogId: string): Answer<RoomLiveView> {
    return this.#http.request(liveViewPath(catalogId), { method: 'GET' }, roomLiveViewSchema);
  }

  updateLiveView(catalogId: string, enabled: boolean): Answer<RoomLiveView> {
    return this.#http.request(
      liveViewPath(catalogId),
      { body: JSON.stringify({ enabled }), method: 'PUT' },
      roomLiveViewSchema,
    );
  }
}

function liveViewPath(catalogId: string): string {
  return `/private-rooms/${encodeURIComponent(catalogId)}/live-view`;
}
