import { Avatar, Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Check, X } from 'lucide-react';
import { useId } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  privateRoomListQueryKey,
  usePrivateRoomAgentKnocks,
} from '@/features/private-rooms/data/private-room-queries';
import { useSubRoomJoinRequests } from '@/features/private-rooms/data/sub-room-queries';
import type { JoinRequestDecision } from '@/features/private-rooms/domain/public-room-join';
import type { SubRoom, SubRoomRef } from '@/features/private-rooms/domain/sub-rooms';
import {
  KNOCK_DIALOG_INTERVAL_MS,
  PrivateRoomAgentKnocks,
} from '@/features/private-rooms/ui/private-room-agent-knocks';
import { formatDateTime } from '@/shared/i18n/formatters';
import { err } from '@/shared/result';

import './sub-room-waiting.css';

/** 说得清的几种没办成；别的说“没办成”，错误码跟在后面。 */
const failureMessages = {
  'private_room.conflict': 'subRooms.settings.requests.failure.conflict',
  'sub_room.archived': 'subRooms.failure.archived',
  'sub_room.not_in_parent': 'subRooms.settings.requests.failure.not_in_parent',
  'sub_room.target_banned': 'subRooms.settings.requests.failure.target_banned',
} as const;

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

export type SubRoomWaitingProps = {
  readonly subRoomRef: SubRoomRef;
  readonly room: SubRoom;
};

/**
 * 子房间设置最上面“在等的”（画布第 8 张）：人的申请（同意、不行）和 Agent 的敲门（让它进来、不让进）。
 * 要批准的子房间没人在等也写一句；别的门没人在等就不出现。开着时隔几秒再问一次。
 */
export function SubRoomWaiting({ room, subRoomRef }: SubRoomWaitingProps) {
  const { i18n, t } = useTranslation();
  const { privateRooms, subRooms } = useAppServices();
  const queryClient = useQueryClient();
  const headingId = useId();
  const requests = useSubRoomJoinRequests(subRooms, subRoomRef, {
    intervalMs: KNOCK_DIALOG_INTERVAL_MS,
  });
  const knocks = usePrivateRoomAgentKnocks(privateRooms, room.catalogId, {
    intervalMs: KNOCK_DIALOG_INTERVAL_MS,
  });
  const answer = useMutation({
    mutationFn: async ({
      decision,
      principalId,
    }: {
      readonly decision: JoinRequestDecision;
      readonly principalId: string;
    }) =>
      subRooms === undefined
        ? err({ code: 'sub_room.unavailable', retryable: false })
        : await subRooms.answerJoinRequest(subRoomRef, principalId, decision),
    onSettled: async () => {
      // 申请、名单上“在等的有几个”和这一间的成员都挂在房间列表的键下面，一起重读。
      await queryClient.invalidateQueries({ queryKey: privateRoomListQueryKey });
    },
  });
  const waiting = requests.data?.ok === true ? requests.data.value : [];
  const knocking = knocks.data?.ok === true ? knocks.data.value.length : 0;
  const count = waiting.length + knocking;
  if (count === 0 && room.access !== 'request') return null;
  const failure = answer.data?.ok === false ? answer.data.error.code : null;
  const busy = answer.isPending ? answer.variables : undefined;
  return (
    <section aria-labelledby={headingId} className="sub-room-waiting">
      <h3 id={headingId}>
        {t('subRooms.settings.requests.title')}{' '}
        <span className="sub-room-waiting__count">{count}</span>
      </h3>
      {count === 0 ? (
        <p className="sub-room-waiting__empty">{t('subRooms.settings.requests.none')}</p>
      ) : null}
      {waiting.length === 0 ? null : (
        <ol className="sub-room-waiting__list">
          {waiting.map((request) => (
            <li key={request.principalId}>
              <Avatar tint="ink">
                {(Array.from(request.displayName.trim())[0] ?? '?').toUpperCase()}
              </Avatar>
              <div className="sub-room-waiting__who">
                <strong>{request.displayName}</strong>
                <small>
                  {t('privateRooms.joinRequests.requestedAt', {
                    date: formatDateTime(request.requestedAtUnixMs, i18n.resolvedLanguage),
                  })}
                </small>
                {request.message === '' ? null : (
                  <p className="sub-room-waiting__message">{request.message}</p>
                )}
              </div>
              <div className="sub-room-waiting__actions">
                <Button
                  aria-label={t('privateRooms.joinRequests.rejectLabel', {
                    name: request.displayName,
                  })}
                  disabled={busy !== undefined}
                  icon={<X aria-hidden="true" />}
                  onClick={() => {
                    answer.mutate({ decision: 'reject', principalId: request.principalId });
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t('privateRooms.joinRequests.reject')}
                </Button>
                <Button
                  aria-busy={
                    busy?.principalId === request.principalId && busy.decision === 'approve'
                  }
                  aria-label={t('privateRooms.joinRequests.approveLabel', {
                    name: request.displayName,
                  })}
                  disabled={busy !== undefined}
                  icon={<Check aria-hidden="true" />}
                  onClick={() => {
                    answer.mutate({ decision: 'approve', principalId: request.principalId });
                  }}
                  size="compact"
                >
                  {t('privateRooms.joinRequests.approve')}
                </Button>
              </div>
            </li>
          ))}
        </ol>
      )}
      {failure === null ? null : (
        <p className="sub-room-waiting__failure" role="alert">
          {knownFailure(failure) ? t(failureMessages[failure]) : t('subRooms.failure.other')}
          {knownFailure(failure) ? null : <code>{failure}</code>}
        </p>
      )}
      <PrivateRoomAgentKnocks
        catalogId={room.catalogId}
        emptySpinner={false}
        rooms={privateRooms}
      />
    </section>
  );
}
