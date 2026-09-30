import { Banner, Details } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';

/** 私人房间的操作没成功：一句话说明，错误码和关联 ID 收进详情。 */
export function PrivateRoomFailureNotice({ failure }: { readonly failure: PrivateRoomFailure }) {
  const { t } = useTranslation();
  return (
    <Banner tone="danger" title={t('privateRooms.failure.title')}>
      <Details summary={t('privateRooms.failure.details')}>
        <code>{failure.code}</code>
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
