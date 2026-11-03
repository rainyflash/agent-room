import { Banner, Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Check, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  privateRoomListQueryKey,
  usePrivateRoomJoinRequests,
} from '@/features/private-rooms/data/private-room-queries';
import type { PrivateRoomListing } from '@/features/private-rooms/domain/private-room-listing';
import type {
  JoinRequestDecision,
  PublicRoomJoinGateway,
} from '@/features/private-rooms/domain/public-room-join';
import { PrivateRoomFailureNotice } from '@/features/private-rooms/ui/private-room-failure-notice';
import { formatDateTime } from '@/shared/i18n/formatters';

export type PrivateRoomJoinRequestsProps = {
  readonly catalogId: string;
  readonly joins: PublicRoomJoinGateway;
  /** 公开设置；还没读到时是 null。要申请的公开房间没人申请时也写一句，别的房间没人申请就不出现。 */
  readonly listing: PrivateRoomListing | null;
};

/**
 * 房间设置“成员”一节下面的“申请”（`specs/public-rooms/design.md`“加入方式”）：房主和管理员看在等的
 * 申请和留的话，批准或拒绝。批准以后申请人的网页自动加入，不用他再点一次。
 */
export function PrivateRoomJoinRequests({
  catalogId,
  joins,
  listing,
}: PrivateRoomJoinRequestsProps) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const requests = usePrivateRoomJoinRequests(joins, catalogId);
  const answer = useMutation({
    mutationFn: async ({
      decision,
      principalId,
    }: {
      readonly decision: JoinRequestDecision;
      readonly principalId: string;
    }) => await joins.answerJoinRequest(catalogId, principalId, decision),
    onSuccess: async (result) => {
      if (!result.ok) return;
      // 申请列表和房间成员都挂在房间列表的键下面，一起重读。
      await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
    },
  });
  const pending = requests.data?.ok === true ? requests.data.value : [];
  const asking = listing?.listed === true && listing.joinMode === 'request';
  if (pending.length === 0 && !asking) return null;
  const failure = answer.data?.ok === false ? answer.data.error : null;

  return (
    <section className="private-room-join-requests">
      <div className="private-room-section-heading">
        <div>
          <h3>{t('privateRooms.joinRequests.title')}</h3>
          <p>{t('privateRooms.joinRequests.detail')}</p>
        </div>
        <span>{pending.length}</span>
      </div>
      {pending.length === 0 ? (
        <p className="private-room-join-requests__empty">{t('privateRooms.joinRequests.none')}</p>
      ) : (
        <ol>
          {pending.map((request) => (
            <li key={request.principalId}>
              <div className="private-room-join-requests__who">
                <strong>{request.displayName}</strong>
                <small>
                  {t('privateRooms.joinRequests.requestedAt', {
                    date: formatDateTime(request.requestedAtUnixMs, i18n.resolvedLanguage),
                  })}
                </small>
              </div>
              {request.message === '' ? null : (
                <p className="private-room-join-requests__message">{request.message}</p>
              )}
              <div className="private-room-member__actions">
                <Button
                  aria-label={t('privateRooms.joinRequests.approveLabel', {
                    name: request.displayName,
                  })}
                  disabled={answer.isPending}
                  icon={<Check aria-hidden="true" />}
                  onClick={() => {
                    answer.mutate({ decision: 'approve', principalId: request.principalId });
                  }}
                  size="compact"
                  tone="primary"
                >
                  {t('privateRooms.joinRequests.approve')}
                </Button>
                <Button
                  aria-label={t('privateRooms.joinRequests.rejectLabel', {
                    name: request.displayName,
                  })}
                  disabled={answer.isPending}
                  icon={<X aria-hidden="true" />}
                  onClick={() => {
                    answer.mutate({ decision: 'reject', principalId: request.principalId });
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t('privateRooms.joinRequests.reject')}
                </Button>
              </div>
            </li>
          ))}
        </ol>
      )}
      {failure === null ? null : failure.code === 'room_join.banned' ? (
        <Banner tone="warning">{t('privateRooms.joinRequests.banned')}</Banner>
      ) : (
        <PrivateRoomFailureNotice failure={failure} />
      )}
    </section>
  );
}
