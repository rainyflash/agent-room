import { CopyBlock, Dialog } from '@agent-room/ui-system';
import { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  SHARE_MESSAGE_EXCERPT_CHARS,
  shareExcerpt,
  type ShareableRoom,
} from '@/features/room-share/domain/room-share';

import { ShareLinkField, ShareQrCode, ShareTargetLinks } from './share-parts';
import { SharePreview } from './share-preview';
import './share-room-dialog.css';

export type ShareRoomDialogProps = {
  readonly room: ShareableRoom;
  readonly onClose: () => void;
  /**
   * 放在最下面的几节：房间页里是绑着仓库时房主看得到的“README 徽章”（3P-8b-1）和分享过的几段对话（3P-7b）；
   * 发现页没有。
   */
  readonly children?: ReactNode;
};

/**
 * 公开房间的“分享”（`specs/public-rooms/design.md`“分享链接”）：链接和复制、一段能改的话、发到 X、微博、
 * Reddit（打开它们自己的分享网址）、系统的分享菜单（有才给）、二维码，和发出去的预览。改了的话只用这一次，
 * 不存。
 */
export function ShareRoomDialog({ children, onClose, room }: ShareRoomDialogProps) {
  const { t } = useTranslation();
  const excerpt = shareExcerpt(room.description, SHARE_MESSAGE_EXCERPT_CHARS);
  const [text, setText] = useState(
    () =>
      `${
        excerpt === null
          ? t('roomShare.message.defaultShort', { name: room.name })
          : t('roomShare.message.default', { excerpt, name: room.name })
      }\n${room.shareUrl}`,
  );
  const messageId = useId();
  const hintId = useId();
  return (
    <Dialog
      className="room-share"
      closeLabel={t('roomShare.close')}
      description={t('roomShare.subtitle')}
      onClose={onClose}
      size="wide"
      title={t('roomShare.title')}
    >
      <div className="room-share__layout">
        <div className="room-share__main">
          <ShareLinkField url={room.shareUrl} />
          <div className="room-share__field">
            <label htmlFor={messageId}>{t('roomShare.message')}</label>
            <textarea
              aria-describedby={hintId}
              id={messageId}
              onChange={(event) => {
                setText(event.target.value);
              }}
              rows={3}
              value={text}
            />
            <div className="room-share__message-footer">
              <small id={hintId}>{t('roomShare.message.hint')}</small>
              <CopyBlock
                copiedLabel={t('roomShare.copied')}
                copyLabel={t('roomShare.copyMessage')}
                failedLabel={t('roomShare.copyFailed')}
                hideText
                size="compact"
                text={text}
                tone="ghost"
              />
            </div>
          </div>
          <ShareTargetLinks room={room} text={text} />
        </div>
        <aside className="room-share__side">
          <ShareQrCode
            detail={t('roomShare.qr.detail')}
            label={t('roomShare.qr.label', { name: room.name })}
            url={room.shareUrl}
          />
          <SharePreview room={room} />
        </aside>
      </div>
      <p className="room-share__note">{t('roomShare.note')}</p>
      {children}
    </Dialog>
  );
}
