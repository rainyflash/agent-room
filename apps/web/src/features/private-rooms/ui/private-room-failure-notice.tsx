import { Banner, Details } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';

/**
 * 私人房间的操作没成功：一句话说明，错误码和关联 ID 收进详情。服务器说“只有房主能做”时把原因写出来：
 * 界面照最新的成员名单已经不给这些按钮，碰得到多半是名单还没跟上（刚被设成管理员）。
 */
export function PrivateRoomFailureNotice({ failure }: { readonly failure: PrivateRoomFailure }) {
  const { t } = useTranslation();
  return (
    <Banner tone="danger" title={t('privateRooms.failure.title')}>
      {failure.code === 'private_room.owner_only' ? (
        <p>{t('privateRooms.failure.ownerOnly')}</p>
      ) : null}
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
