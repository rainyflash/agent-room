import { Button } from '@agent-room/ui-system';
import { AtSign, Reply, Send, X } from 'lucide-react';
import type { RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import type { ConversationParticipant } from '@/features/conversation/domain/conversation';
import type { useConversationComposer } from '@/features/conversation/ui/use-conversation-composer';
import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';

export type ConversationComposerController = ReturnType<typeof useConversationComposer>;

export function ConversationComposer({
  composer,
  roomId,
  participants,
  names,
  canSend,
  writesAllowed,
  input,
}: {
  readonly composer: ConversationComposerController;
  readonly roomId: string;
  readonly participants: readonly ConversationParticipant[];
  readonly names: ReadonlyMap<string, string>;
  readonly canSend: boolean;
  readonly writesAllowed: boolean;
  readonly input: RefObject<HTMLTextAreaElement | null>;
}) {
  const { t } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const { publication } = composer;
  const canEdit = composer.editable && runtime.writes.allowed && writesAllowed;
  const pending = publication.matches('unknown') || publication.matches('acceptedBindingPending');
  const submitting = publication.matches('publishing') || publication.matches('reconciling');
  const failed = publication.matches('failed');
  const unavailable = publication.matches('identityUnavailable');
  return (
    <form
      className="conversation-panel__composer"
      onSubmit={(event) => {
        event.preventDefault();
        if (canSend) composer.submit();
      }}
    >
      {composer.reply === null ? null : (
        <div className="conversation-panel__reply">
          <Reply aria-hidden="true" />
          <span>{t('conversation.replying', { name: composer.reply.actor.displayName })}</span>
          <button
            type="button"
            disabled={!canEdit}
            aria-label={t('conversation.cancelReply')}
            onClick={composer.cancelReply}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      )}
      {composer.mentions.length === 0 ? null : (
        <div className="conversation-panel__mentions">
          {composer.mentions.map((id) => (
            <button
              type="button"
              key={id}
              disabled={!canEdit}
              aria-label={t('conversation.removeMention', { name: names.get(id) ?? id })}
              onClick={() => {
                composer.removeMention(id);
              }}
            >
              @{names.get(id) ?? id}
              <X aria-hidden="true" />
            </button>
          ))}
        </div>
      )}
      <label className="sr-only" htmlFor={`chat-${roomId}`}>
        {t('conversation.input')}
      </label>
      <textarea
        id={`chat-${roomId}`}
        ref={input}
        rows={3}
        maxLength={8_000}
        placeholder={t('conversation.placeholder')}
        value={composer.text}
        disabled={!canEdit}
        onChange={(event) => {
          composer.changeText(event.target.value);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            if (canSend) composer.submit();
          }
        }}
      />
      <div className="conversation-panel__tools">
        <label>
          <AtSign aria-hidden="true" />
          <span className="sr-only">{t('conversation.mention')}</span>
          <select
            aria-label={t('conversation.mention')}
            value=""
            disabled={!canEdit || composer.mentions.length >= 8}
            onChange={(event) => {
              if (event.target.value) {
                composer.mention(event.target.value);
                input.current?.focus();
              }
            }}
          >
            <option value="">{t('conversation.mention')}</option>
            {participants
              .filter((participant) => !composer.mentions.includes(participant.matrixUserId))
              .map((participant) => (
                <option key={participant.matrixUserId} value={participant.matrixUserId}>
                  {participant.displayName}
                </option>
              ))}
          </select>
        </label>
        <span>{t('conversation.count', { count: Array.from(composer.text).length })}</span>
        <Button type="submit" disabled={!canSend} icon={<Send aria-hidden="true" />} size="compact">
          {t(submitting ? 'conversation.sending' : 'conversation.send')}
        </Button>
      </div>
      {composer.text.length > 0 && !composer.valid ? (
        <p role="alert">{t('conversation.invalid')}</p>
      ) : null}
      {publication.matches('published') ? <p role="status">{t('conversation.sent')}</p> : null}
      {pending ? (
        <div role="status">
          <p>{t('conversation.pending')}</p>
          <Button onClick={composer.reconcile} size="compact">
            {t('conversation.check')}
          </Button>
        </div>
      ) : null}
      {failed ? (
        <div role="alert">
          <p>{t('conversation.failed')}</p>
          {publication.context.failure?.retryable ? (
            <Button onClick={composer.retry} size="compact">
              {t('conversation.retry')}
            </Button>
          ) : null}
          {publication.can({ type: 'CLOSE' }) ? (
            <Button onClick={composer.edit} size="compact">
              {t('conversation.edit')}
            </Button>
          ) : null}
        </div>
      ) : null}
      {unavailable ? (
        <div role="alert">
          <p>{t('conversation.unavailable')}</p>
          <Button onClick={composer.retryIdentity} size="compact">
            {t('conversation.retry')}
          </Button>
        </div>
      ) : null}
    </form>
  );
}
