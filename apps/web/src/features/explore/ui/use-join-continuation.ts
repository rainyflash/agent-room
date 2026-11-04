import { useNavigate } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';

import {
  joinContinuationStep,
  type ExploreFailure,
  type ExploreRoomDetail,
} from '@/features/explore/domain/explore';
import type { SessionStateName } from '@/features/session/ui/connection-model';
import type { Result } from '@/shared/result';

import type { ExploreActions } from './use-explore-actions';

/** 从分享页点“加入”过来的（`?join=1`）；`signup` 是点了“用邮箱注册”。 */
export type JoinContinuation = { readonly signup: boolean };

/**
 * 从分享页点“加入”过来（`specs/public-rooms/design.md`“分享链接”）：没登录的先去登录（点了“用邮箱注册”
 * 的去注册），回来停在 `?join=1` 上；等登录和消息都连好、读到这间房你的处境，照 `joinContinuationStep`
 * 接着做：已经在里面的直接进房间，直接进的、收到过邀请的加入，要申请的把光标放进申请框。只做一次。
 *
 * 去登录之前先把 `join` 从地址上去掉：从登录页后退回来停在详情上，不再被送回登录页。登录完回来的那次
 * 要等消息也登录好（那一步还会再跳一次，回来的地址得带着 `join`）才去掉。
 */
export function useJoinContinuation({
  continuation,
  detail,
  onHandled,
  onJoin,
  onSignIn,
  registrationOpen,
  selectedCode,
  sessionState,
  signedIn,
}: {
  readonly continuation: JoinContinuation | null;
  readonly detail: Result<ExploreRoomDetail, ExploreFailure> | undefined;
  readonly onHandled: (() => void) | undefined;
  readonly onJoin: ExploreActions['onJoin'];
  readonly onSignIn: (shareCode: string, intent: 'register' | 'sign-in') => void;
  readonly registrationOpen: boolean;
  readonly selectedCode: string | null;
  readonly sessionState: SessionStateName;
  readonly signedIn: boolean;
}): boolean {
  const navigate = useNavigate();
  const handled = useRef(false);
  const [focusRequest, setFocusRequest] = useState(false);
  useEffect(() => {
    if (continuation === null || selectedCode === null || handled.current) return;
    if (sessionState === 'unauthenticated' && !signedIn) {
      handled.current = true;
      onHandled?.();
      onSignIn(selectedCode, continuation.signup && registrationOpen ? 'register' : 'sign-in');
      return;
    }
    if (sessionState !== 'ready' || !signedIn || detail === undefined) return;
    handled.current = true;
    if (!detail.ok) {
      onHandled?.();
      return;
    }
    const room = detail.value;
    switch (joinContinuationStep(room)) {
      case 'enter':
        void navigate({
          params: { catalogId: room.catalogId },
          replace: true,
          search: {},
          to: '/lobby/$catalogId',
        });
        return;
      case 'join':
        onHandled?.();
        onJoin(room);
        return;
      case 'request':
        onHandled?.();
        setFocusRequest(true);
        return;
      case 'none':
        onHandled?.();
    }
  }, [
    continuation,
    detail,
    navigate,
    onHandled,
    onJoin,
    onSignIn,
    registrationOpen,
    selectedCode,
    sessionState,
    signedIn,
  ]);
  return focusRequest;
}
