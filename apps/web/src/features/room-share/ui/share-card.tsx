import { Button } from '@agent-room/ui-system';
import { Check, Copy, Download, Share2 } from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  SHARE_CARD_SIZE,
  shareCardFileName,
  type ShareCardContent,
} from '@/features/room-share/domain/share-card';
import { saveBlob } from '@/shared/browser/save-file';

import { renderShareCard } from './share-card-canvas';
import './share-room-dialog.css';

export type ShareCardProps = {
  /** 卡片上写什么。用的地方要记住这份内容（`useMemo`）：换一份就重画一张。 */
  readonly content: ShareCardContent;
  readonly heading: string;
  /** 读屏读的：这张图上有什么。 */
  readonly alt: string;
  /** 图下面的一句：这张图发出去意味着什么。 */
  readonly detail: string;
};

/**
 * 发给人的那张卡片（`specs/share-card/design.md`）：在这台设备上画好，能复制、存下来，手机上能直接交给
 * 系统的分享菜单。画不出来（没有画布）时这一块不出现，链接照旧能发。
 */
export function ShareCard({ alt, content, detail, heading }: ShareCardProps) {
  const { t } = useTranslation();
  const headingId = useId();
  const card = useDrawnCard(content);
  const [copied, setCopied] = useState<{
    readonly blob: Blob;
    readonly state: 'copied' | 'failed';
  } | null>(null);
  if (card.kind === 'failed') return null;
  const blob = card.kind === 'ready' ? card.blob : null;
  const fileName = shareCardFileName(content.roomName);
  const file = blob === null ? null : shareableFile(blob, fileName);
  // 换了一张图，上一张的“已复制”不算。
  const copyState = copied !== null && copied.blob === blob ? copied.state : 'idle';
  return (
    <section aria-labelledby={headingId} className="share-card">
      <h3 id={headingId}>{heading}</h3>
      <figure aria-busy={blob === null ? true : undefined} className="share-card__figure">
        {card.kind === 'ready' ? (
          <img
            alt={alt}
            height={SHARE_CARD_SIZE.height}
            src={card.url}
            width={SHARE_CARD_SIZE.width}
          />
        ) : null}
      </figure>
      <div className="share-card__actions">
        {canCopyImages() ? (
          <Button
            disabled={blob === null}
            icon={
              copyState === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />
            }
            onClick={() => {
              if (blob === null) return;
              // 要在这一下点击里交给剪贴板：等别的事做完，有的浏览器就不认这次点击了。
              navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]).then(
                () => {
                  setCopied({ blob, state: 'copied' });
                },
                () => {
                  setCopied({ blob, state: 'failed' });
                },
              );
            }}
            size="compact"
            tone="ghost"
          >
            {t(copyState === 'copied' ? 'roomShare.card.copied' : 'roomShare.card.copy')}
          </Button>
        ) : null}
        <Button
          disabled={blob === null}
          icon={<Download aria-hidden="true" />}
          onClick={() => {
            if (blob !== null) saveBlob(blob, fileName);
          }}
          size="compact"
          tone="ghost"
        >
          {t('roomShare.card.save')}
        </Button>
        {file === null ? null : (
          <Button
            icon={<Share2 aria-hidden="true" />}
            onClick={() => {
              // 关掉系统的分享菜单也算失败，不用说。
              navigator.share({ files: [file] }).catch(() => undefined);
            }}
            size="compact"
            tone="ghost"
          >
            {t('roomShare.card.send')}
          </Button>
        )}
      </div>
      {copyState === 'failed' ? <p role="status">{t('roomShare.card.copyFailed')}</p> : null}
      <p>{detail}</p>
    </section>
  );
}

type DrawnCard =
  | { readonly kind: 'drawing' }
  | { readonly kind: 'failed' }
  | { readonly kind: 'ready'; readonly blob: Blob; readonly url: string };

type Drawn = {
  readonly content: ShareCardContent;
  /** `null` 是没画成。 */
  readonly image: { readonly blob: Blob; readonly url: string } | null;
};

/** 画这一份内容的卡片；内容换了重画，关掉时收回做好的地址。 */
function useDrawnCard(content: ShareCardContent): DrawnCard {
  const [drawn, setDrawn] = useState<Drawn | null>(null);
  useEffect(() => {
    let stopped = false;
    let url: string | null = null;
    void renderShareCard(content).then(
      (blob) => {
        if (stopped) return;
        url = URL.createObjectURL(blob);
        setDrawn({ content, image: { blob, url } });
      },
      () => {
        if (!stopped) setDrawn({ content, image: null });
      },
    );
    return () => {
      stopped = true;
      if (url !== null) URL.revokeObjectURL(url);
    };
  }, [content]);
  // 上一份内容画的不算：它的地址已经收回了。
  if (drawn?.content !== content) return { kind: 'drawing' };
  return drawn.image === null ? { kind: 'failed' } : { kind: 'ready', ...drawn.image };
}

/** 这个浏览器能不能把图放进剪贴板：旧一点的没有 `ClipboardItem`，不安全的页面没有剪贴板。 */
function canCopyImages(): boolean {
  return 'clipboard' in navigator && typeof ClipboardItem === 'function';
}

/** 系统的分享菜单收不收图片：手机上一般收，电脑上多半不收。收才给“发给…”。 */
function shareableFile(blob: Blob, fileName: string): File | null {
  if (typeof navigator.canShare !== 'function' || typeof File !== 'function') return null;
  try {
    const file = new File([blob], fileName, { type: blob.type });
    return navigator.canShare({ files: [file] }) ? file : null;
  } catch {
    return null;
  }
}
