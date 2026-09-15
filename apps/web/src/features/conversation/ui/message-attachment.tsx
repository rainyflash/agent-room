import { useEffect, useRef, useState } from 'react';
import { Download, Image, Paperclip } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useOptionalAppServices } from '@/app/app-services';
import type { MessageContentReference } from '@/features/messages/domain/message';
import { attachmentSize, imageAttachment } from '../domain/conversation-attachment';

export function MessageAttachment({
  content,
  roomId,
  name,
}: {
  readonly content: MessageContentReference;
  readonly roomId: string;
  readonly name: string;
}) {
  const { t } = useTranslation();
  const services = useOptionalAppServices();
  const [state, setState] = useState<'idle' | 'loading' | 'failed' | 'ready'>('idle');
  const [url, setUrl] = useState<string | null>(null);
  const generation = useRef(0);
  const ownedUrl = useRef<string | null>(null);
  useEffect(() => {
    generation.current += 1;
    setState('idle');
    setUrl(null);
    return () => {
      generation.current += 1;
      if (ownedUrl.current !== null) URL.revokeObjectURL(ownedUrl.current);
      ownedUrl.current = null;
    };
  }, [content.contentId, roomId]);
  const load = async () => {
    if (services === null || state === 'loading') return;
    const attempt = ++generation.current;
    setState('loading');
    try {
      const ticket = await services.content.issueReadTicket(content.contentId);
      if (!ticket.ok) throw new Error(ticket.error.code);
      const downloaded = await services.content.download(content.contentId, ticket.value.ticket);
      if (!downloaded.ok) throw new Error(downloaded.error.code);
      const verified = await services.contentVerifier.verify(downloaded.value, content, roomId);
      if (!verified.ok) throw new Error(verified.error.code);
      if (attempt !== generation.current) return;
      // Only raster images render inline; HTML, SVG and other files remain downloads.
      const next = URL.createObjectURL(
        new Blob([Uint8Array.from(verified.value.bytes)], {
          type: imageAttachment(content.mediaType) ? content.mediaType : 'application/octet-stream',
        }),
      );
      ownedUrl.current = next;
      setUrl(next);
      setState('ready');
    } catch {
      if (attempt === generation.current) setState('failed');
    }
  };
  return (
    <div className="conversation-attachment">
      <div>
        <Paperclip aria-hidden="true" />
        <span>
          <strong>{name}</strong>
          <small>
            {attachmentSize(content.sizeBytes)} · {content.mediaType}
          </small>
        </span>
      </div>
      {url !== null ? (
        <>
          {imageAttachment(content.mediaType) ? <img src={url} alt={name} loading="lazy" /> : null}
          <a href={url} download={name}>
            <Download aria-hidden="true" />
            {t('attachments.download')}
          </a>
        </>
      ) : (
        <button
          type="button"
          disabled={services === null || state === 'loading'}
          onClick={() => void load()}
        >
          {imageAttachment(content.mediaType) ? (
            <Image aria-hidden="true" />
          ) : (
            <Download aria-hidden="true" />
          )}
          {t(
            state === 'loading'
              ? 'attachments.loading'
              : state === 'failed'
                ? 'attachments.retry'
                : imageAttachment(content.mediaType)
                  ? 'attachments.preview'
                  : 'attachments.load',
          )}
        </button>
      )}
      {state === 'failed' ? <p role="alert">{t('attachments.failed')}</p> : null}
    </div>
  );
}
