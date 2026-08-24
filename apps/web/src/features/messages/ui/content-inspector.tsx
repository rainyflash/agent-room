import { useMachine } from '@xstate/react';
import { Button } from '@agent-room/ui-system';
import {
  Bot,
  Download,
  Eye,
  FileCheck2,
  LoaderCircle,
  RotateCw,
  ShieldAlert,
  ShieldCheck,
  ShieldX,
  X,
  type LucideIcon,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { HandoffGateway } from '@/features/handoffs/domain/handoff';
import { HandoffPanel } from '@/features/handoffs/ui/handoff-panel';
import { createContentInspectionMachine } from '@/features/messages/application/content-inspection-machine';
import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { MessageSignatureStatus, RoomMessageSignal } from '@/features/messages/domain/message';
import { RestrictedMarkdown } from '@/features/messages/ui/restricted-markdown';

const signatureIconByStatus: Readonly<Record<MessageSignatureStatus, LucideIcon>> = Object.freeze({
  instance_verified: ShieldCheck,
  matrix_sender_matched: ShieldAlert,
  revoked_after_event: ShieldX,
});

export type ContentInspectorProps = {
  readonly contentGateway: ContentGateway;
  readonly contentVerifier: ContentVerifier;
  readonly handoffGateway: HandoffGateway;
  readonly message: RoomMessageSignal;
  readonly onClose: () => void;
};

export function ContentInspector({
  contentGateway,
  contentVerifier,
  handoffGateway,
  message,
  onClose,
}: ContentInspectorProps) {
  const { i18n, t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const machine = useMemo(
    () => createContentInspectionMachine({ content: contentGateway, verifier: contentVerifier }),
    [contentGateway, contentVerifier],
  );
  const [inspection, send] = useMachine(machine);
  const [handoffOpen, setHandoffOpen] = useState(false);
  const createdAt = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(message.serverTimestamp);
  const preview = message.preview;
  const reference = message.content;
  const content = inspection.context.content;
  const SignatureIcon = signatureIconByStatus[message.signatureStatus];
  const busy =
    inspection.matches('requestingTicket') ||
    inspection.matches('downloading') ||
    inspection.matches('verifying');

  return (
    <motion.aside
      animate={{ opacity: 1, x: 0 }}
      aria-labelledby="content-inspector-title"
      className="content-inspector"
      exit={reduceMotion === true ? { opacity: 0 } : { opacity: 0, x: 24 }}
      initial={reduceMotion === true ? false : { opacity: 0, x: 32 }}
      transition={{ bounce: 0.1, damping: 28, stiffness: 240, type: 'spring' }}
    >
      <header className="content-inspector__header">
        <div>
          <p className="eyebrow">{t('messages.inspector.eyebrow')}</p>
          <h2 id="content-inspector-title">
            {preview?.title ?? t(`messages.lifecycle.${message.lifecycle}`)}
          </h2>
        </div>
        <button
          aria-label={t('messages.inspector.close')}
          className="inspector-close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="content-inspector__source">
        <span className="message-signal__author" aria-hidden="true">
          {initials(message.actor.displayName)}
        </span>
        <div>
          <strong>{message.actor.displayName}</strong>
          <span>{t(`messages.provenance.${message.actor.provenance}`)}</span>
        </div>
        <time dateTime={new Date(message.serverTimestamp).toISOString()}>{createdAt}</time>
      </div>
      <dl className="content-inspector__trust">
        <div>
          <dt>{t('messages.inspector.signature')}</dt>
          <dd data-signature-status={message.signatureStatus}>
            <SignatureIcon aria-hidden="true" />
            <span>{t(`messages.signature.${message.signatureStatus}`)}</span>
          </dd>
        </div>
        <div>
          <dt>{t('messages.inspector.room')}</dt>
          <dd>
            <code title={message.roomId}>{message.roomId}</code>
          </dd>
        </div>
      </dl>
      {preview === null || reference === null ? (
        <div className="content-inspector__terminal">
          <ShieldAlert aria-hidden="true" />
          <p>{t('messages.inspector.contentUnavailable')}</p>
        </div>
      ) : (
        <>
          <section className="content-inspector__preview" aria-label={t('messages.preview.label')}>
            <p>{preview.summary}</p>
            <dl>
              <div>
                <dt>{t('messages.preview.type')}</dt>
                <dd>{reference.mediaType}</dd>
              </div>
              <div>
                <dt>{t('messages.preview.size')}</dt>
                <dd>{formatBytes(reference.sizeBytes, i18n.resolvedLanguage)}</dd>
              </div>
              <div>
                <dt>{t('messages.preview.sensitivity')}</dt>
                <dd>{t(`messages.sensitivity.${preview.sensitivity}`)}</dd>
              </div>
            </dl>
            {preview.riskFlags.length === 0 ? null : (
              <div className="content-inspector__risks" role="note">
                <ShieldAlert aria-hidden="true" />
                <div>
                  <strong>{t('messages.preview.risks')}</strong>
                  <p>{preview.riskFlags.join(' · ')}</p>
                </div>
              </div>
            )}
          </section>
          {inspection.matches('idle') ? (
            <div className="content-inspector__consent">
              <p>{t('messages.inspector.consent')}</p>
              <Button
                icon={<Eye aria-hidden="true" />}
                onClick={() => {
                  send({
                    request: {
                      matrixEventId: message.matrixEventId,
                      messageId: message.messageId,
                      reference,
                    },
                    type: 'OPEN',
                  });
                }}
                tone="primary"
              >
                {t('messages.inspector.open')}
              </Button>
            </div>
          ) : null}
          {busy ? (
            <div aria-live="polite" className="content-inspector__progress" role="status">
              <LoaderCircle aria-hidden="true" />
              <div>
                <strong>{t(`messages.inspector.stage.${String(inspection.value)}`)}</strong>
                <p>{t('messages.inspector.stageDetail')}</p>
              </div>
            </div>
          ) : null}
          {inspection.matches('failed') ? (
            <div className="content-inspector__failure" role="alert">
              <ShieldAlert aria-hidden="true" />
              <div>
                <strong>{t('messages.inspector.failed')}</strong>
                <p>{t(`messages.failure.${inspection.context.failure?.code ?? 'unknown'}`)}</p>
                {inspection.context.failure?.correlationId === undefined ? null : (
                  <code>{inspection.context.failure.correlationId}</code>
                )}
              </div>
              <Button
                icon={<RotateCw aria-hidden="true" />}
                onClick={() => {
                  send({ type: 'RETRY' });
                }}
                size="compact"
                tone="quiet"
              >
                {t('messages.inspector.retry')}
              </Button>
            </div>
          ) : null}
          {inspection.matches('ready') && content !== null ? (
            <>
              <section
                className="content-inspector__verified"
                aria-label={t('messages.body.label')}
              >
                <header>
                  <FileCheck2 aria-hidden="true" />
                  <div>
                    <strong>{t('messages.body.verified')}</strong>
                    <span>{content.digestSha256.slice(0, 16)}…</span>
                  </div>
                </header>
                {content.mode === 'text' && content.text !== undefined ? (
                  content.mediaType === 'text/markdown' ? (
                    <RestrictedMarkdown source={content.text} />
                  ) : (
                    <pre>{content.text}</pre>
                  )
                ) : (
                  <div className="content-inspector__attachment">
                    <p>{t('messages.body.attachmentNotice')}</p>
                    <Button
                      icon={<Download aria-hidden="true" />}
                      onClick={() => {
                        downloadVerifiedContent(
                          content.bytes,
                          content.mediaType,
                          reference.contentId,
                        );
                      }}
                      tone="quiet"
                    >
                      {t('messages.body.download')}
                    </Button>
                  </div>
                )}
                {content.mediaType === 'text/markdown' ? (
                  <p className="content-inspector__sandbox-note">
                    {t('messages.body.safeMarkdown')}
                  </p>
                ) : null}
              </section>
              <section className="content-inspector__handoff-gate">
                <Bot aria-hidden="true" />
                <div>
                  <strong>{t('handoff.gate.title')}</strong>
                  <p>{t('handoff.gate.detail')}</p>
                </div>
                <Button
                  disabled={handoffOpen}
                  onClick={() => {
                    setHandoffOpen(true);
                  }}
                  size="compact"
                  tone="network"
                >
                  {t('handoff.gate.open')}
                </Button>
              </section>
              {handoffOpen ? (
                <HandoffPanel
                  gateway={handoffGateway}
                  message={{ ...message, content: reference, preview }}
                  onBack={() => {
                    setHandoffOpen(false);
                  }}
                />
              ) : null}
            </>
          ) : null}
        </>
      )}
    </motion.aside>
  );
}

function initials(displayName: string): string {
  return [...displayName.trim()].slice(0, 2).join('').toUpperCase();
}

function formatBytes(value: number, language: string | undefined): string {
  const formatter = new Intl.NumberFormat(language, { maximumFractionDigits: 1 });
  if (value < 1_024) {
    return `${formatter.format(value)} B`;
  }
  if (value < 1_024 * 1_024) {
    return `${formatter.format(value / 1_024)} KiB`;
  }
  return `${formatter.format(value / (1_024 * 1_024))} MiB`;
}

function downloadVerifiedContent(bytes: Uint8Array, mediaType: string, contentId: string): void {
  const ownedBytes = Uint8Array.from(bytes);
  const url = URL.createObjectURL(new Blob([ownedBytes.buffer], { type: mediaType }));
  const anchor = document.createElement('a');
  anchor.download = `${contentId}.${extensionFor(mediaType)}`;
  anchor.href = url;
  anchor.rel = 'noopener';
  anchor.click();
  URL.revokeObjectURL(url);
}

function extensionFor(mediaType: string): string {
  const extensions: Readonly<Record<string, string>> = {
    'application/json': 'json',
    'text/markdown': 'md',
    'text/plain': 'txt',
  };
  return extensions[mediaType] ?? 'bin';
}
