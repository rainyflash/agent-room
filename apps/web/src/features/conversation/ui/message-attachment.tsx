import { useEffect, useRef, useState } from 'react';
import { Download, Image, Paperclip } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useOptionalAppServices } from '@/app/app-services';
import { desktopSavesFile } from '@/features/desktop/domain/saved-file-types';
import { FileSaveNote } from '@/features/desktop/ui/file-save-note';
import { useFileSave } from '@/features/desktop/ui/use-file-save';
import type { MessageContentReference } from '@/features/messages/domain/message';
import { attachmentSize, imageAttachment } from '../domain/conversation-attachment';

/** 取回来、核对过的附件：`blob` 交给桌面应用存，`url` 给图片和浏览器里的下载链接用。 */
type LoadedAttachment = { readonly blob: Blob; readonly url: string };

/**
 * 消息里的一个附件：先取回来核对，图片就地显示，别的存成文件。
 *
 * 存成文件在浏览器里是一个下载链接，交给浏览器自己的下载。桌面端的窗口里页面自己的下载不出声
 * （Windows 上悄悄存进“下载”，Mac 上直接被取消），所以那里由应用自己存（`useFileSave`），存完说存到哪了：
 * 不是图片的点一次“下载文件”就取回来、存好；应用只存点开不会运行的几种，别的照实说存不了。
 */
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
  const saver = useFileSave();
  const [state, setState] = useState<'idle' | 'loading' | 'failed' | 'ready'>('idle');
  const [loaded, setLoaded] = useState<LoadedAttachment | null>(null);
  const generation = useRef(0);
  const ownedUrl = useRef<string | null>(null);
  useEffect(() => {
    generation.current += 1;
    setState('idle');
    setLoaded(null);
    return () => {
      generation.current += 1;
      if (ownedUrl.current !== null) URL.revokeObjectURL(ownedUrl.current);
      ownedUrl.current = null;
    };
  }, [content.contentId, roomId]);
  const image = imageAttachment(content.mediaType);
  const load = async (): Promise<Blob | null> => {
    if (services === null || state === 'loading') return null;
    const attempt = ++generation.current;
    setState('loading');
    try {
      const ticket = await services.content.issueReadTicket(content.contentId);
      if (!ticket.ok) throw new Error(ticket.error.code);
      const downloaded = await services.content.download(content.contentId, ticket.value.ticket);
      if (!downloaded.ok) throw new Error(downloaded.error.code);
      const verified = await services.contentVerifier.verify(downloaded.value, content, roomId);
      if (!verified.ok) throw new Error(verified.error.code);
      if (attempt !== generation.current) return null;
      // Only raster images render inline; HTML, SVG and other files remain downloads.
      const blob = new Blob([Uint8Array.from(verified.value.bytes)], {
        type: image ? content.mediaType : 'application/octet-stream',
      });
      const url = URL.createObjectURL(blob);
      ownedUrl.current = url;
      setLoaded({ blob, url });
      setState('ready');
      return blob;
    } catch {
      if (attempt === generation.current) setState('failed');
      return null;
    }
  };
  // 桌面端不是图片的附件：一个按钮，取回来接着就存。再点一次存的是已经取回来的那一份。
  const fetchAndSave = async () => {
    const blob = loaded?.blob ?? (await load());
    if (blob !== null) saver.save(name, blob);
  };
  const saved = loaded !== null && saver.last?.content === loaded.blob ? saver.last : null;
  const saving = saved?.kind === 'saving';
  const loading = state === 'loading';
  const unsupported = saver.native && !desktopSavesFile(name);
  const unsupportedNote = (
    <p className="conversation-attachment__note">{t('attachments.desktopUnsupported')}</p>
  );
  const saveLabel = t(saving ? 'attachments.savingFile' : 'attachments.download');
  let action;
  if (saver.native && !image) {
    action = unsupported ? (
      unsupportedNote
    ) : (
      <button
        type="button"
        disabled={services === null || loading || saving}
        onClick={() => void fetchAndSave()}
      >
        <Download aria-hidden="true" />
        {loading
          ? t('attachments.loading')
          : state === 'failed'
            ? t('attachments.retry')
            : saveLabel}
      </button>
    );
  } else if (loaded === null) {
    action = (
      <button type="button" disabled={services === null || loading} onClick={() => void load()}>
        {image ? <Image aria-hidden="true" /> : <Download aria-hidden="true" />}
        {t(
          loading
            ? 'attachments.loading'
            : state === 'failed'
              ? 'attachments.retry'
              : image
                ? 'attachments.preview'
                : 'attachments.load',
        )}
      </button>
    );
  } else if (!saver.native) {
    action = (
      <a href={loaded.url} download={name}>
        <Download aria-hidden="true" />
        {t('attachments.download')}
      </a>
    );
  } else {
    action = unsupported ? (
      unsupportedNote
    ) : (
      <button
        type="button"
        disabled={saving}
        onClick={() => {
          saver.save(name, loaded.blob);
        }}
      >
        <Download aria-hidden="true" />
        {saveLabel}
      </button>
    );
  }
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
      {image && loaded !== null ? <img src={loaded.url} alt={name} loading="lazy" /> : null}
      {action}
      {state === 'failed' ? <p role="alert">{t('attachments.failed')}</p> : null}
      <FileSaveNote onShow={saved === null ? null : saver.show} save={saved} />
    </div>
  );
}
