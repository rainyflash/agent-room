import { Button, Dialog, Spinner } from '@agent-room/ui-system';
import { RefreshCw } from 'lucide-react';
import { useId, useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import type { RoomMessageSignal } from '@/features/messages/domain/message';
import {
  excerptFailureKind,
  excerptFailureRetryable,
  excerptPreviewLine,
} from '@/features/room-excerpts/domain/room-excerpts';
import {
  ShareLinkField,
  ShareQrCode,
  ShareTargetLinks,
} from '@/features/room-share/ui/share-parts';
import { formatDayAndTime } from '@/shared/i18n/formatters';

import { ExcerptFailureNotice } from './excerpt-failure';
import type { ExcerptShareState } from './excerpt-selection';
import './room-excerpts.css';

/**
 * “分享这一段”的对话框（`specs/redesign/project-rooms.md`“分享一段对话”）：上面是链接和“复制链接”、发到 X、
 * 微博、Reddit，旁边二维码，和房间的“分享”是同一套；下面预览这几句，照分享出去的那一页的样子（不公开的只说
 * 一句、正文开头 280 个字、附件只说有、改过的标出来）。点“分享”时就向控制面要了链接，这里先说正在生成。
 */
export function ShareExcerptDialog({
  roomName,
  messages,
  state,
  onRetry,
  onClose,
}: {
  readonly roomName: string;
  readonly messages: readonly RoomMessageSignal[];
  readonly state: ExcerptShareState;
  readonly onRetry: () => void;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation();
  const shared = state.kind === 'shared' ? state.url : null;
  return (
    <Dialog
      className="room-share room-excerpt-share"
      closeLabel={t('roomShare.close')}
      description={t('roomExcerpt.dialog.subtitle')}
      onClose={onClose}
      size="wide"
      title={t('roomExcerpt.dialog.title')}
    >
      <div className={`room-share__layout${shared === null ? ' room-share__layout--single' : ''}`}>
        <div className="room-share__main">
          {state.kind === 'pending' ? (
            <p className="room-excerpt-share__creating" role="status">
              <Spinner />
              {t('roomExcerpt.dialog.creating')}
            </p>
          ) : state.kind === 'failed' ? (
            <ExcerptFailureNotice action="share" failure={state.failure}>
              {excerptFailureRetryable(excerptFailureKind(state.failure)) ? (
                <Button
                  icon={<RefreshCw aria-hidden="true" />}
                  onClick={onRetry}
                  size="compact"
                  tone="primary"
                >
                  {t('roomExcerpt.dialog.retry')}
                </Button>
              ) : (
                <Button onClick={onClose} size="compact" tone="ghost">
                  {t('roomExcerpt.dialog.reselect')}
                </Button>
              )}
            </ExcerptFailureNotice>
          ) : (
            <>
              <ShareLinkField url={state.url} />
              <ShareTargetLinks
                room={{ name: roomName, shareUrl: state.url }}
                text={t('roomExcerpt.dialog.message', { name: roomName })}
              />
            </>
          )}
          <ExcerptPreview messages={messages} />
        </div>
        {shared === null ? null : (
          <aside className="room-share__side">
            <ShareQrCode
              detail={t('roomExcerpt.dialog.qr.detail')}
              label={t('roomExcerpt.dialog.qr.label')}
              url={shared}
            />
          </aside>
        )}
      </div>
      <p className="room-share__note">{t('roomExcerpt.dialog.note')}</p>
    </Dialog>
  );
}

/** 要分享的这几句：谁、人还是 Agent、几点，正文照分享出去的那一页。 */
function ExcerptPreview({ messages }: { readonly messages: readonly RoomMessageSignal[] }) {
  const { t, i18n } = useTranslation();
  const headingId = useId();
  const language = i18n.resolvedLanguage;
  const time = useMemo(
    () => new Intl.DateTimeFormat(language, { hour: '2-digit', minute: '2-digit' }),
    [language],
  );
  const first = messages[0];
  return (
    <section aria-labelledby={headingId} className="excerpt-preview">
      <h3 id={headingId}>
        {t('roomExcerpt.list.item', {
          count: messages.length,
          date:
            first === undefined
              ? ''
              : formatDayAndTime(first.serverTimestamp, Date.now(), language),
        })}
      </h3>
      {/* 最多 30 句：框住、能滚，键盘也滚得动。 */}
      <ol aria-labelledby={headingId} className="excerpt-preview__list" tabIndex={0}>
        {messages.map((message) => {
          const line = excerptPreviewLine(message);
          return (
            <li key={message.messageId}>
              <p className="excerpt-preview__meta">
                <strong>{message.actor.displayName}</strong>
                <span>
                  {t(message.actor.kind === 'human' ? 'conversation.human' : 'conversation.agent')}
                </span>
                <time dateTime={new Date(message.serverTimestamp).toISOString()}>
                  {time.format(message.serverTimestamp)}
                </time>
              </p>
              {line.kind === 'withheld' ? (
                <p className="excerpt-preview__text excerpt-preview__text--quiet">
                  {t('roomExcerpt.preview.withheld')}
                </p>
              ) : (
                <>
                  {line.text === '' ? null : (
                    <p className="excerpt-preview__text">
                      {line.truncated ? `${line.text}…` : line.text}
                      {message.edited ? (
                        <span className="excerpt-preview__edited">
                          {` ${t('roomExcerpt.preview.edited')}`}
                        </span>
                      ) : null}
                    </p>
                  )}
                  {line.attachment ? (
                    <p className="excerpt-preview__text excerpt-preview__text--quiet">
                      {t('roomExcerpt.preview.attachment')}
                    </p>
                  ) : null}
                </>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
