import { Button, CopyBlock, Dialog } from '@agent-room/ui-system';
import { Share2 } from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  SHARE_MESSAGE_EXCERPT_CHARS,
  SHARE_PREVIEW_CHARS,
  shareExcerpt,
  shareTargets,
  withoutLink,
  type ShareableRoom,
  type ShareTargets,
} from '@/features/room-share/domain/room-share';

import { QrCode } from './qr-code';
import './share-room-dialog.css';

export type ShareRoomDialogProps = {
  readonly room: ShareableRoom;
  readonly onClose: () => void;
};

const targets = ['x', 'weibo', 'reddit'] as const satisfies readonly (keyof ShareTargets)[];

/**
 * 公开房间的“分享”（`specs/public-rooms/design.md`“分享链接”）：链接和复制、一段能改的话、发到 X、微博、
 * Reddit（打开它们自己的分享网址）、系统的分享菜单（有才给）、二维码，和发出去的预览。改了的话只用这一次，
 * 不存。
 */
export function ShareRoomDialog({ onClose, room }: ShareRoomDialogProps) {
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
  const linkId = useId();
  const messageId = useId();
  const hintId = useId();
  const postToId = useId();
  const copyLabels = {
    copiedLabel: t('roomShare.copied'),
    failedLabel: t('roomShare.copyFailed'),
  };
  const links = shareTargets(room, text);
  const systemShare = typeof navigator.share === 'function';
  return (
    <Dialog
      className="room-share"
      closeLabel={t('roomShare.close')}
      description={t('roomShare.subtitle')}
      icon={<Share2 />}
      onClose={onClose}
      size="wide"
      title={t('roomShare.title')}
    >
      <div className="room-share__layout">
        <div className="room-share__main">
          <div className="room-share__field">
            <label htmlFor={linkId}>{t('roomShare.link')}</label>
            <div className="room-share__link">
              <input
                id={linkId}
                onFocus={(event) => {
                  event.currentTarget.select();
                }}
                readOnly
                value={room.shareUrl}
              />
              <CopyBlock
                {...copyLabels}
                copyLabel={t('roomShare.copyLink')}
                hideText
                size="compact"
                text={room.shareUrl}
              />
            </div>
          </div>
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
                {...copyLabels}
                copyLabel={t('roomShare.copyMessage')}
                hideText
                size="compact"
                text={text}
                tone="ghost"
              />
            </div>
          </div>
          <section aria-labelledby={postToId} className="room-share__field">
            <h3 id={postToId}>{t('roomShare.postTo')}</h3>
            <div className="room-share__targets">
              {targets.map((target) => (
                <a
                  aria-label={t('roomShare.targetLabel', {
                    target: t(`roomShare.target.${target}`),
                  })}
                  className="ar-button ar-button--compact ar-button--ghost"
                  href={links[target]}
                  key={target}
                  rel="noreferrer"
                  target="_blank"
                >
                  {t(`roomShare.target.${target}`)}
                </a>
              ))}
              {systemShare ? (
                <Button
                  onClick={() => {
                    // 关掉系统的分享菜单也算失败，不用说。
                    navigator
                      .share({
                        text: withoutLink(text, room.shareUrl),
                        title: `${room.name} · Agent Room`,
                        url: room.shareUrl,
                      })
                      .catch(() => undefined);
                  }}
                  size="compact"
                  tone="ghost"
                >
                  {t('roomShare.more')}
                </Button>
              ) : null}
            </div>
          </section>
        </div>
        <aside className="room-share__side">
          <section className="room-share__qr">
            <h3>{t('roomShare.qr')}</h3>
            <QrCode label={t('roomShare.qr.label', { name: room.name })} value={room.shareUrl} />
            <p>{t('roomShare.qr.detail')}</p>
          </section>
          <SharePreview room={room} />
        </aside>
      </div>
      <p className="room-share__note">{t('roomShare.note')}</p>
    </Dialog>
  );
}

/** 发到社交媒体上的样子：和分享页的 `og:` 标签一样（通用的预览图、“房间名 · Agent Room”、简介开头）。 */
function SharePreview({ room }: { readonly room: ShareableRoom }) {
  const { t } = useTranslation();
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="room-share__preview">
      <h3 id={headingId}>{t('roomShare.preview')}</h3>
      <figure>
        <img alt="" height={640} src="/social-preview.png" width={1280} />
        <figcaption>
          <small>{new URL(room.shareUrl).host}</small>
          <strong>{`${room.name} · Agent Room`}</strong>
          <span>
            {shareExcerpt(room.description, SHARE_PREVIEW_CHARS) ?? t('roomShare.preview.generic')}
          </span>
        </figcaption>
      </figure>
    </section>
  );
}
