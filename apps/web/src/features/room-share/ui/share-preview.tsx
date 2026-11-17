import { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import {
  SHARE_PREVIEW_CHARS,
  shareExcerpt,
  sharePreviewImage,
  type SharePreviewGateway,
  type ShareableRoom,
} from '@/features/room-share/domain/room-share';

/**
 * 发到社交媒体上的样子：和分享页的 `og:` 标签一样，“房间名 · Agent Room”、简介开头，图是这间房自己的那张
 * （3P-8b-2，房间名和现在几个在线）。要不到时（还没公开、服务器忙、连不上、没有分享码）放通用图。
 */
export function SharePreview({ room }: { readonly room: ShareableRoom }) {
  const { t } = useTranslation();
  const headingId = useId();
  const image = useRoomPreviewImage(useOptionalAppServices()?.sharePreviews, room);
  return (
    <section aria-labelledby={headingId} className="room-share__preview">
      <h3 id={headingId}>{t('roomShare.preview')}</h3>
      <figure aria-busy={image.kind === 'loading' ? true : undefined}>
        {image.kind === 'loading' ? (
          <div className="room-share__preview-placeholder" />
        ) : (
          <img
            alt=""
            data-preview={image.kind}
            height={640}
            onError={image.kind === 'room' ? image.failed : undefined}
            src={image.src}
            width={1280}
          />
        )}
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

type PreviewImage =
  | { readonly kind: 'loading' }
  | { readonly kind: 'room'; readonly src: string; readonly failed: () => void }
  | { readonly kind: 'generic'; readonly src: string };

/** 这一次要的是哪张：分享码和房间名变了就重新要（名字在图上）。 */
type Loaded = {
  readonly key: string;
  /** 这间房自己的图做成的 `blob:` 地址；`null` 是要不到，放通用图。 */
  readonly url: string | null;
};

/**
 * 去要这间房自己的图，做成 `blob:` 地址。对话框关掉、换了房间时停下没回来的请求，收回做好的地址。要着的
 * 那一会儿先空着一块同样大小的地方，免得通用图闪一下再换。
 */
function useRoomPreviewImage(
  gateway: SharePreviewGateway | undefined,
  room: Pick<ShareableRoom, 'name' | 'shareCode'>,
): PreviewImage {
  const generic = sharePreviewImage(room.name);
  const key = `${room.shareCode ?? ''}\n${room.name}`;
  const asks = gateway !== undefined && room.shareCode !== null;
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [broken, setBroken] = useState<string | null>(null);
  const { shareCode } = room;
  useEffect(() => {
    if (gateway === undefined || shareCode === null) return;
    const controller = new AbortController();
    let url: string | null = null;
    void gateway.roomImage(shareCode, controller.signal).then(
      (image) => {
        if (controller.signal.aborted) return;
        url = image === null ? null : URL.createObjectURL(image);
        setLoaded({ key, url });
      },
      () => {
        if (!controller.signal.aborted) setLoaded({ key, url: null });
      },
    );
    return () => {
      controller.abort();
      if (url !== null) URL.revokeObjectURL(url);
    };
  }, [gateway, key, shareCode]);
  // 上一次要的（换了名字、换了房间）不算：它的地址已经收回了。
  const current = loaded?.key === key ? loaded : null;
  if (!asks) return { kind: 'generic', src: generic };
  if (current === null) return { kind: 'loading' };
  if (current.url === null || broken === current.url) return { kind: 'generic', src: generic };
  const { url } = current;
  return {
    kind: 'room',
    src: url,
    failed: () => {
      setBroken(url);
    },
  };
}
