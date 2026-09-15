import { useEffect, useRef, useState } from 'react';
import { Paperclip, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  attachmentSize,
  imageAttachment,
  maximumAttachmentBytes,
} from '../domain/conversation-attachment';
import type { ConversationComposerController } from './conversation-composer';
import type { AttachmentFailure } from '../domain/conversation-attachment';

export async function selectAttachment(
  files: FileList | readonly File[],
  composer: ConversationComposerController,
): Promise<AttachmentFailure | 'oneFile' | null> {
  if (files.length !== 1) return 'oneFile';
  const file = files[0];
  if (file === undefined) return null;
  if (file.size > maximumAttachmentBytes) return 'tooLarge';
  if (file.size === 0) return 'empty';
  try {
    // Browser filenames are not paths; normalize control characters before publishing metadata.
    const name = Array.from(file.name.replace(/[\p{Cc}/\\]/gu, '_'))
      .slice(0, 120)
      .join('');
    await composer.attach({
      name,
      mediaType: file.type.toLowerCase() || 'application/octet-stream',
      bytes: new Uint8Array(await file.arrayBuffer()),
    });
    return null;
  } catch {
    return 'readFailed';
  }
}

export function AttachmentPicker({
  composer,
  disabled,
  onFailure,
}: {
  readonly composer: ConversationComposerController;
  readonly disabled: boolean;
  readonly onFailure: (failure: AttachmentFailure | 'oneFile' | null) => void;
}) {
  const { t } = useTranslation();
  const input = useRef<HTMLInputElement>(null);
  const attachment = composer.attachment;
  const file = attachment.kind === 'ready' ? attachment.file : null;
  const [preview, setPreview] = useState<string | null>(null);
  useEffect(() => {
    if (file === null || !imageAttachment(file.mediaType)) {
      setPreview(null);
      return;
    }
    const url = URL.createObjectURL(new Blob([file.bytes], { type: file.mediaType }));
    setPreview(url);
    return () => {
      URL.revokeObjectURL(url);
    };
  }, [file]);
  return (
    <div className="conversation-attachment-picker">
      <input
        ref={input}
        className="sr-only"
        type="file"
        tabIndex={-1}
        aria-label={t('attachments.choose')}
        disabled={disabled}
        onChange={(event) => {
          const files = event.target.files;
          if (files?.length) void selectAttachment(files, composer).then(onFailure);
          event.target.value = '';
        }}
      />
      <button
        type="button"
        disabled={disabled || attachment.kind === 'loading'}
        onClick={() => input.current?.click()}
      >
        <Paperclip aria-hidden="true" />
        {t('attachments.choose')}
      </button>
      <small>{t('attachments.limit')}</small>
      {attachment.kind === 'none' ? null : (
        <div className="conversation-attachment-picker__file">
          {preview === null ? <Paperclip aria-hidden="true" /> : <img src={preview} alt="" />}
          <span>
            <strong>{attachment.reference.name}</strong>
            <small>
              {attachment.kind === 'loading'
                ? t('attachments.saving')
                : attachment.kind === 'missing'
                  ? t('attachments.missing')
                  : attachmentSize(attachment.file.bytes.byteLength)}
            </small>
          </span>
          <button
            type="button"
            disabled={disabled}
            aria-label={t('attachments.remove')}
            onClick={composer.removeAttachment}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      )}
    </div>
  );
}
