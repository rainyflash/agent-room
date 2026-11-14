import { useTranslation } from 'react-i18next';

/**
 * 对话版面里，主房间来了几条你还没看到的消息（你在私聊里，或者在看资料）。
 * 放在按钮里用：按钮自己写 `aria-label`，名字不变；条数经 `aria-describedby` 指到这里读出来。
 */
export function RoomUnreadCount({ count, id }: { readonly count: number; readonly id: string }) {
  const { t } = useTranslation();
  return (
    <span className="room-unread" id={id}>
      <span aria-hidden="true">{count > 99 ? '99+' : count}</span>
      <span className="sr-only">{t('roomGame.unread', { count })}</span>
    </span>
  );
}
