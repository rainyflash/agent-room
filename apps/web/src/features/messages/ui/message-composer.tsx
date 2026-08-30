import { useMachine } from '@xstate/react';
import { Button, StatusMark } from '@agent-room/ui-system';
import {
  BadgeCheck,
  CircleAlert,
  LoaderCircle,
  MessageSquarePlus,
  Minus,
  Radio,
  RefreshCw,
  Send,
  ShieldCheck,
  X,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useMemo, useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  BrowserSubmissionIdFactory,
  type MessageSubmissionIdFactory,
} from '@/features/messages/adapters/browser-submission-id-factory';
import { createMessagePublicationMachine } from '@/features/messages/application/message-publication-machine';
import {
  inspectPublicationRisks,
  type MessagePublicationDraft,
  type MessagePublisher,
  type PublicationRiskFlag,
  validatePublicationDraft,
} from '@/features/messages/domain/publication';
import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';
import type { TranslationKey } from '@/shared/i18n/resources';

const browserSubmissionIds = new BrowserSubmissionIdFactory();

const publicationStateMessages = [
  ['resolvingIdentity', 'messages.composer.state.resolvingIdentity'],
  ['identityUnavailable', 'messages.composer.state.identityUnavailable'],
  ['ready', 'messages.composer.state.ready'],
  ['publishing', 'messages.composer.state.publishing'],
  ['acceptedBindingPending', 'messages.composer.state.acceptedBindingPending'],
  ['unknown', 'messages.composer.state.unknown'],
  ['reconciling', 'messages.composer.state.reconciling'],
  ['failed', 'messages.composer.state.failed'],
  ['published', 'messages.composer.state.published'],
] as const satisfies readonly (readonly [string, TranslationKey])[];

const publicationRiskMessageKey: Readonly<Record<PublicationRiskFlag, TranslationKey>> = {
  external_links: 'messages.composer.risk.external_links',
  html_markup: 'messages.composer.risk.html_markup',
};

type EditableDraft = Omit<MessagePublicationDraft, 'language' | 'riskFlags'> & {
  readonly language: string;
};

export type MessageComposerProps = {
  readonly publisher: MessagePublisher;
  readonly roomId: string;
  readonly roomName: string;
  readonly submissionIds?: MessageSubmissionIdFactory;
};

export function MessageComposer({
  publisher,
  roomId,
  roomName,
  submissionIds = browserSubmissionIds,
}: MessageComposerProps) {
  const { i18n, t } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const reduceMotion = useReducedMotion();
  const machine = useMemo(() => createMessagePublicationMachine(publisher), [publisher]);
  const [publication, send] = useMachine(machine);
  const [minimized, setMinimized] = useState(true);
  const [draft, setDraft] = useState<EditableDraft>(() => emptyDraft(i18n.resolvedLanguage));
  const riskFlags = useMemo(() => inspectPublicationRisks(draft.body), [draft.body]);
  const draftIssues = useMemo(
    () => validatePublicationDraft({ ...draft, riskFlags }),
    [draft, riskFlags],
  );
  const stateMessageKey =
    publicationStateMessages.find(([state]) => publication.matches(state))?.[1] ??
    'messages.composer.state.failed';

  const openComposer = (): void => {
    if (publication.matches('closed')) {
      send({ roomId, type: 'OPEN' });
    }
    setMinimized(false);
  };
  const minimizeOrClose = (): void => {
    if (publication.can({ type: 'CLOSE' })) {
      send({ type: 'CLOSE' });
    }
    setMinimized(true);
  };
  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!runtime.writes.allowed || !publication.matches('ready') || draftIssues.length > 0) {
      return;
    }
    send({
      request: {
        ...draft,
        ...(draft.language.trim().length === 0 ? {} : { language: draft.language.trim() }),
        riskFlags,
        roomId,
        submissionId: submissionIds.next(),
      },
      type: 'SUBMIT',
    });
  };

  if (minimized) {
    const pending = !publication.matches('closed');
    return (
      <motion.button
        aria-label={t(pending ? 'messages.composer.resumeLabel' : 'messages.composer.openLabel')}
        className={`message-composer-launcher${pending ? ' message-composer-launcher--pending' : ''}`}
        onClick={openComposer}
        type="button"
        {...(reduceMotion === true ? {} : { whileTap: { scale: 0.96 } })}
      >
        {pending ? <StatusMark label={t(stateMessageKey)} tone="alert" /> : null}
        <MessageSquarePlus aria-hidden="true" />
        <span>
          {pending
            ? t('messages.composer.resume', {
                state: t(stateMessageKey),
              })
            : t('messages.composer.open')}
        </span>
      </motion.button>
    );
  }

  return (
    <motion.aside
      animate={{ opacity: 1, scale: 1, x: 0 }}
      aria-labelledby="message-composer-title"
      className="message-composer"
      initial={reduceMotion === true ? false : { opacity: 0, scale: 0.98, x: -12 }}
      transition={{ bounce: 0.08, damping: 27, stiffness: 260, type: 'spring' }}
    >
      <header className="message-composer__header">
        <div>
          <p className="eyebrow">{t('messages.composer.eyebrow')}</p>
          <h2 id="message-composer-title">{t('messages.composer.title')}</h2>
        </div>
        <button
          aria-label={t(
            publication.can({ type: 'CLOSE' })
              ? 'messages.composer.close'
              : 'messages.composer.minimize',
          )}
          className="inspector-close"
          onClick={minimizeOrClose}
          type="button"
        >
          {publication.can({ type: 'CLOSE' }) ? (
            <X aria-hidden="true" />
          ) : (
            <Minus aria-hidden="true" />
          )}
        </button>
      </header>
      <div className="message-composer__target">
        <Radio aria-hidden="true" />
        <div>
          <span>{t('messages.composer.target')}</span>
          <strong>{roomName}</strong>
        </div>
        <code>{roomId}</code>
      </div>
      {publication.context.identity === null ? null : (
        <div className="message-composer__identity">
          <ShieldCheck aria-hidden="true" />
          <div>
            <span>{t('messages.composer.identitySource')}</span>
            <strong>{publication.context.identity.displayName}</strong>
          </div>
          <span>{t('messages.composer.identity.humanSession')}</span>
        </div>
      )}
      {publication.matches('resolvingIdentity') ? (
        <ComposerProgress
          detail={t('messages.composer.identityDetail')}
          label={t('messages.composer.state.resolvingIdentity')}
        />
      ) : null}
      {publication.matches('identityUnavailable') ? (
        <ComposerBoundary
          detail={t(`messages.failure.${publication.context.failure?.code ?? 'unknown'}`)}
          label={t('messages.composer.identityUnavailable')}
        >
          {publication.context.failure?.retryable === true ? (
            <Button
              icon={<RefreshCw aria-hidden="true" />}
              onClick={() => {
                send({ type: 'RETRY_IDENTITY' });
              }}
              size="compact"
              tone="quiet"
            >
              {t('messages.composer.retryIdentity')}
            </Button>
          ) : null}
        </ComposerBoundary>
      ) : null}
      {publication.matches('ready') ? (
        <form
          aria-disabled={!runtime.writes.allowed}
          className="message-composer__form"
          onSubmit={submit}
        >
          {runtime.writes.allowed ? null : (
            <ComposerBoundary
              detail={t(`pwa.writeBlocked.${runtime.writes.reason}`)}
              label={t('pwa.writeBlocked.title')}
            />
          )}
          <label>
            <span>{t('messages.composer.field.title')}</span>
            <input
              maxLength={120}
              onChange={(event) => {
                setDraft((current) => ({ ...current, title: event.target.value }));
              }}
              placeholder={t('messages.composer.field.titlePlaceholder')}
              required
              value={draft.title}
            />
          </label>
          <label>
            <span>{t('messages.composer.field.summary')}</span>
            <textarea
              maxLength={500}
              onChange={(event) => {
                setDraft((current) => ({ ...current, summary: event.target.value }));
              }}
              placeholder={t('messages.composer.field.summaryPlaceholder')}
              required
              rows={2}
              value={draft.summary}
            />
          </label>
          <label>
            <span>{t('messages.composer.field.body')}</span>
            <textarea
              className="message-composer__body"
              onChange={(event) => {
                setDraft((current) => ({ ...current, body: event.target.value }));
              }}
              placeholder={t('messages.composer.field.bodyPlaceholder')}
              required
              rows={7}
              value={draft.body}
            />
          </label>
          <div className="message-composer__options">
            <label>
              <span>{t('messages.composer.field.language')}</span>
              <input
                maxLength={35}
                onChange={(event) => {
                  setDraft((current) => ({ ...current, language: event.target.value }));
                }}
                value={draft.language}
              />
            </label>
            <label>
              <span>{t('messages.composer.field.sensitivity')}</span>
              <select
                onChange={(event) => {
                  setDraft((current) => ({
                    ...current,
                    sensitivity: event.target.value as EditableDraft['sensitivity'],
                  }));
                }}
                value={draft.sensitivity}
              >
                <option value="normal">{t('messages.sensitivity.normal')}</option>
                <option value="sensitive">{t('messages.sensitivity.sensitive')}</option>
                <option value="restricted">{t('messages.sensitivity.restricted')}</option>
              </select>
            </label>
          </div>
          <div className="message-composer__risk" role="note">
            <CircleAlert aria-hidden="true" />
            <div>
              <strong>{t('messages.composer.risk.title')}</strong>
              <p>
                {riskFlags.length === 0
                  ? t('messages.composer.risk.clear')
                  : riskFlags.map((flag) => t(publicationRiskMessageKey[flag])).join(' · ')}
              </p>
            </div>
          </div>
          <footer className="message-composer__footer">
            <div aria-live="polite">
              {draftIssues.length === 0
                ? t('messages.composer.ready')
                : t('messages.composer.issueCount', { count: draftIssues.length })}
            </div>
            <Button
              disabled={draftIssues.length > 0 || !runtime.writes.allowed}
              icon={<Send aria-hidden="true" />}
              tone="primary"
              type="submit"
            >
              {t('messages.composer.send')}
            </Button>
          </footer>
        </form>
      ) : null}
      {publication.matches('publishing') ? (
        <ComposerProgress
          detail={t('messages.composer.publishDetail')}
          label={t(`messages.composer.progress.${publication.context.progress ?? 'uploading'}`)}
        />
      ) : null}
      {publication.matches('reconciling') ? (
        <ComposerProgress
          detail={t('messages.composer.reconcileDetail')}
          label={t('messages.composer.state.reconciling')}
        />
      ) : null}
      {publication.matches('unknown') ? (
        <ComposerBoundary
          detail={t('messages.composer.unknownDetail')}
          label={t('messages.composer.state.unknown')}
        >
          <Button
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => {
              send({ type: 'RECONCILE' });
            }}
            size="compact"
            tone="quiet"
          >
            {t('messages.composer.queryStatus')}
          </Button>
        </ComposerBoundary>
      ) : null}
      {publication.matches('acceptedBindingPending') ? (
        <ComposerBoundary
          detail={t('messages.composer.bindingDetail')}
          label={t('messages.composer.state.acceptedBindingPending')}
        >
          <Button
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => {
              send({ type: 'RECONCILE' });
            }}
            size="compact"
            tone="quiet"
          >
            {t('messages.composer.queryStatus')}
          </Button>
        </ComposerBoundary>
      ) : null}
      {publication.matches('failed') ? (
        <ComposerBoundary
          detail={t(`messages.failure.${publication.context.failure?.code ?? 'unknown'}`)}
          label={t('messages.composer.state.failed')}
        >
          {publication.context.failure?.retryable === true && publication.can({ type: 'RETRY' }) ? (
            <Button
              icon={<RefreshCw aria-hidden="true" />}
              onClick={() => {
                send({ type: 'RETRY' });
              }}
              size="compact"
              tone="quiet"
            >
              {t(
                publication.context.recovery === 'reconcile'
                  ? 'messages.composer.queryAgain'
                  : 'messages.composer.retrySameSubmission',
              )}
            </Button>
          ) : null}
        </ComposerBoundary>
      ) : null}
      {publication.matches('published') ? (
        <section className="message-composer__success" role="status">
          <BadgeCheck aria-hidden="true" />
          <div>
            <strong>{t('messages.composer.state.published')}</strong>
            <p>{t('messages.composer.publishedDetail')}</p>
          </div>
          <Button
            onClick={() => {
              setDraft(emptyDraft(i18n.resolvedLanguage));
              send({ type: 'RESET' });
            }}
            size="compact"
            tone="quiet"
          >
            {t('messages.composer.newMessage')}
          </Button>
        </section>
      ) : null}
    </motion.aside>
  );
}

function ComposerProgress({ detail, label }: { readonly detail: string; readonly label: string }) {
  return (
    <section aria-live="polite" className="message-composer__progress" role="status">
      <LoaderCircle aria-hidden="true" />
      <div>
        <strong>{label}</strong>
        <p>{detail}</p>
      </div>
    </section>
  );
}

function ComposerBoundary({
  children,
  detail,
  label,
}: {
  readonly children?: ReactNode;
  readonly detail: string;
  readonly label: string;
}) {
  return (
    <section className="message-composer__boundary" role="status">
      <CircleAlert aria-hidden="true" />
      <div>
        <strong>{label}</strong>
        <p>{detail}</p>
      </div>
      {children}
    </section>
  );
}

function emptyDraft(language: string | undefined): EditableDraft {
  return {
    body: '',
    language: language?.startsWith('zh') === true ? 'zh-CN' : 'en',
    mediaType: 'text/markdown',
    sensitivity: 'normal',
    summary: '',
    title: '',
  };
}
