import { Dialog } from '@agent-room/ui-system';
import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  SHARE_MESSAGE_EXCERPT_CHARS,
  shareExcerpt,
  type ShareableRoom,
} from '@/features/room-share/domain/room-share';
import type { ShareCardContent } from '@/features/room-share/domain/share-card';
import { roomTone } from '@/shared/ui/room-tone';

import { ShareCard } from './share-card';
import { ShareLinkField, ShareMessageField, ShareTargetLinks } from './share-parts';
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
 * Reddit（打开它们自己的分享网址）、系统的分享菜单（有才给）、发出去的预览，旁边是一张带二维码的分享卡片
 * （`specs/share-card/design.md`），能扫、能复制、能存下来发。改了的话只用这一次，不存。
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
  const card = useMemo<ShareCardContent>(
    () => ({
      action: t('roomShare.card.action'),
      lead: t('roomShare.card.lead'),
      notes: [t('roomShare.card.noApp'), t('roomShare.card.signIn')],
      roomName: room.name,
      tagline: t('roomShare.card.tagline'),
      tone: roomTone(room.catalogId),
      url: room.shareUrl,
    }),
    [room.catalogId, room.name, room.shareUrl, t],
  );
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
          <ShareMessageField onChange={setText} text={text} />
          <ShareTargetLinks room={room} text={text} />
          <SharePreview room={room} />
        </div>
        <aside className="room-share__side">
          <ShareCard
            alt={t('roomShare.card.alt', { name: room.name })}
            content={card}
            detail={t('roomShare.card.detail')}
            heading={t('roomShare.card.title')}
          />
        </aside>
      </div>
      <p className="room-share__note">{t('roomShare.note')}</p>
      {children}
    </Dialog>
  );
}
