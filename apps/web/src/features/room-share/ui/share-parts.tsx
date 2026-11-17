import { Button, CopyBlock } from '@agent-room/ui-system';
import { useId } from 'react';
import { useTranslation } from 'react-i18next';

import {
  shareTargets,
  withoutLink,
  type ShareableRoom,
  type ShareTargets,
} from '@/features/room-share/domain/room-share';

import { QrCode } from './qr-code';
import './share-room-dialog.css';

const targets = ['x', 'weibo', 'reddit'] as const satisfies readonly (keyof ShareTargets)[];

/** 链接一行：选得中的地址和“复制链接”。房间的“分享”和分享一段对话共用。 */
export function ShareLinkField({ url }: { readonly url: string }) {
  const { t } = useTranslation();
  const linkId = useId();
  return (
    <div className="room-share__field">
      <label htmlFor={linkId}>{t('roomShare.link')}</label>
      <div className="room-share__link">
        <input
          id={linkId}
          onFocus={(event) => {
            event.currentTarget.select();
          }}
          readOnly
          value={url}
        />
        <CopyBlock
          copiedLabel={t('roomShare.copied')}
          copyLabel={t('roomShare.copyLink')}
          failedLabel={t('roomShare.copyFailed')}
          hideText
          size="compact"
          text={url}
        />
      </div>
    </div>
  );
}

/**
 * 发到 X、微博、Reddit（打开它们自己的分享网址，不加载它们的脚本）和系统的分享菜单（有才给）。`text` 是
 * 跟着链接一起发的那句话，没写字时用 `room.name`。
 */
export function ShareTargetLinks({
  room,
  text,
}: {
  readonly room: ShareableRoom;
  readonly text: string;
}) {
  const { t } = useTranslation();
  const postToId = useId();
  const links = shareTargets(room, text);
  const systemShare = typeof navigator.share === 'function';
  return (
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
  );
}

/** 链接的二维码，在本机画；底下一句说怎么扫。 */
export function ShareQrCode({
  url,
  label,
  detail,
}: {
  readonly url: string;
  readonly label: string;
  readonly detail: string;
}) {
  const { t } = useTranslation();
  return (
    <section className="room-share__qr">
      <h3>{t('roomShare.qr')}</h3>
      <QrCode label={label} value={url} />
      <p>{detail}</p>
    </section>
  );
}
