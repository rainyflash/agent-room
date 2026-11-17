import { Banner, Details } from '@agent-room/ui-system';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import {
  EXCERPT_MAX_MESSAGES,
  excerptFailureKind,
  excerptRetryMinutes,
} from '@/features/room-excerpts/domain/room-excerpts';

/** 分享、删掉没成时那一句大白话；错误码和关联 ID 只在“详情”里。 */
export function useExcerptFailureMessage(): (
  failure: PrivateRoomFailure,
  action: 'share' | 'delete',
) => string {
  const { t } = useTranslation();
  return (failure, action) => {
    const kind = excerptFailureKind(failure);
    switch (kind) {
      case 'not_live':
        return t('roomExcerpt.error.not_live');
      case 'too_long':
        return t('roomExcerpt.error.too_long', { max: EXCERPT_MAX_MESSAGES });
      case 'before_live':
        return t('roomExcerpt.error.before_live');
      case 'range_not_found':
        return t('roomExcerpt.error.range_not_found');
      case 'rate_limited': {
        const minutes = excerptRetryMinutes(failure);
        return minutes === null
          ? t('roomExcerpt.error.rate_limitedLater')
          : t('roomExcerpt.error.rate_limited', { count: minutes });
      }
      case 'not_found':
        return t(
          action === 'delete' ? 'roomExcerpt.deleteError.not_found' : 'roomExcerpt.error.not_found',
        );
      case 'forbidden':
        return t(
          action === 'delete' ? 'roomExcerpt.deleteError.forbidden' : 'roomExcerpt.error.forbidden',
        );
      case 'unavailable':
        return t('roomExcerpt.error.unavailable');
      case 'invalid':
        return t('roomExcerpt.error.invalid');
      case 'invalid_origin':
        return t('roomExcerpt.error.invalid_origin');
      case 'unreachable':
        return t('roomExcerpt.error.unreachable');
      case 'other':
        return t('roomExcerpt.error.other');
    }
  };
}

export function ExcerptFailureNotice({
  failure,
  action,
  children,
}: {
  readonly failure: PrivateRoomFailure;
  readonly action: 'share' | 'delete';
  /** 跟在说明后面的按钮（再试一次、回去改选）。 */
  readonly children?: ReactNode;
}) {
  const { t } = useTranslation();
  const message = useExcerptFailureMessage();
  return (
    <Banner
      className="room-excerpts__failure"
      role="alert"
      title={t(action === 'delete' ? 'roomExcerpt.deleteError.title' : 'roomExcerpt.error.title')}
      tone="warning"
    >
      <p>{message(failure, action)}</p>
      {children === undefined ? null : <div className="room-excerpts__actions">{children}</div>}
      <Details summary={t('roomExcerpt.details')}>
        <code>{failure.code}</code>
        {failure.reason === undefined ? null : (
          <>
            {' · '}
            <code>{failure.reason}</code>
          </>
        )}
        {failure.correlationId === undefined ? null : (
          <>
            {' · '}
            <code>{failure.correlationId}</code>
          </>
        )}
      </Details>
    </Banner>
  );
}
