import { Button } from '@agent-room/ui-system';
import { CircleAlert, Flag, LoaderCircle, ShieldCheck, X } from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import { moderationCaseListQueryKey } from '@/features/moderation/data/moderation-queries';
import {
  moderationReasons,
  type ModerationGateway,
  type ModerationReason,
} from '@/features/moderation/domain/moderation';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';

export type MessageReportControlProps = {
  readonly catalogId: string;
  readonly gateway: ModerationGateway;
  readonly message: RoomMessageSignal;
};

export function MessageReportControl({ catalogId, gateway, message }: MessageReportControlProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const reduceMotion = useReducedMotion();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const launcherRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const descriptionId = useId();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState<ModerationReason>('other');
  const [description, setDescription] = useState('');
  const [includePreview, setIncludePreview] = useState(false);
  const mutation = useMutation({
    mutationFn: async () =>
      await gateway.report(identifiers.next(), {
        description,
        evidence: {
          endToEndEncrypted: message.endToEndEncrypted,
          matrixEventId: message.matrixEventId,
          ...(includePreview && message.preview !== null
            ? { reporterSubmittedExcerpt: message.preview.summary }
            : {}),
          roomCatalogId: catalogId,
        },
        reason,
        targetKind: 'event',
        targetReference: message.matrixEventId,
      }),
    onSuccess: async (result) => {
      if (result.ok) {
        await queryClient.invalidateQueries({ queryKey: moderationCaseListQueryKey });
      }
    },
  });
  const result = mutation.data ?? null;

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    closeRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !mutation.isPending) {
        setOpen(false);
        launcherRef.current?.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [mutation.isPending, open]);

  const close = (): void => {
    if (mutation.isPending) {
      return;
    }
    setOpen(false);
    launcherRef.current?.focus();
  };

  const begin = (): void => {
    mutation.reset();
    setDescription('');
    setIncludePreview(false);
    setReason('other');
    setOpen(true);
  };

  return (
    <>
      <Button
        icon={<Flag aria-hidden="true" />}
        onClick={begin}
        ref={launcherRef}
        size="compact"
        tone="ghost"
      >
        {t('moderation.report.action')}
      </Button>
      {createPortal(
        <AnimatePresence>
          {open ? (
            <motion.div
              animate={{ opacity: 1 }}
              className="moderation-dialog-overlay"
              exit={{ opacity: 0 }}
              initial={{ opacity: 0 }}
              key="message-report-dialog"
              onMouseDown={(event) => {
                if (event.target === event.currentTarget) {
                  close();
                }
              }}
              transition={{ duration: reduceMotion ? 0 : 0.14 }}
            >
              <motion.section
                animate={{ opacity: 1, scale: 1, y: 0 }}
                aria-labelledby="message-report-title"
                aria-modal="true"
                className="moderation-report-dialog"
                exit={{ opacity: 0, scale: reduceMotion ? 1 : 0.985, y: 8 }}
                initial={reduceMotion ? false : { opacity: 0, scale: 0.985, y: 18 }}
                role="dialog"
                transition={{ damping: 30, stiffness: 360, type: 'spring' }}
              >
                <header className="moderation-dialog-header">
                  <div>
                    <p className="eyebrow">{t('moderation.report.eyebrow')}</p>
                    <h2 id="message-report-title">{t('moderation.report.title')}</h2>
                    <p>{t('moderation.report.detail')}</p>
                  </div>
                  <button
                    aria-label={t('moderation.report.close')}
                    className="inspector-close"
                    disabled={mutation.isPending}
                    onClick={close}
                    ref={closeRef}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </header>
                {result?.ok === true ? (
                  <div className="moderation-report-success" role="status">
                    <ShieldCheck aria-hidden="true" />
                    <div>
                      <strong>{t('moderation.report.success')}</strong>
                      <p>{t('moderation.report.caseId', { id: result.value.caseId })}</p>
                    </div>
                    <Button onClick={close} tone="primary">
                      {t('moderation.report.close')}
                    </Button>
                  </div>
                ) : (
                  <form
                    className="moderation-report-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      mutation.mutate();
                    }}
                  >
                    <label>
                      <span>{t('moderation.report.reason')}</span>
                      <select
                        disabled={mutation.isPending}
                        onChange={(event) => {
                          setReason(parseReason(event.currentTarget.value));
                        }}
                        value={reason}
                      >
                        {moderationReasons.map((option) => (
                          <option key={option} value={option}>
                            {t(`moderation.reason.${option}`)}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label htmlFor={descriptionId}>
                      <span>{t('moderation.report.description')}</span>
                      <textarea
                        disabled={mutation.isPending}
                        id={descriptionId}
                        maxLength={4_096}
                        onChange={(event) => setDescription(event.currentTarget.value)}
                        placeholder={t('moderation.report.descriptionPlaceholder')}
                        rows={4}
                        value={description}
                      />
                    </label>
                    <label className="moderation-evidence-choice">
                      <input
                        checked={includePreview}
                        disabled={mutation.isPending || message.preview === null}
                        onChange={(event) => setIncludePreview(event.currentTarget.checked)}
                        type="checkbox"
                      />
                      <span>
                        <strong>{t('moderation.report.includePreview')}</strong>
                        <small>{t('moderation.report.includePreviewDetail')}</small>
                      </span>
                    </label>
                    {message.endToEndEncrypted ? (
                      <p className="moderation-encryption-note">
                        <ShieldCheck aria-hidden="true" />
                        {t('moderation.report.encrypted')}
                      </p>
                    ) : null}
                    {result?.ok === false ? (
                      <div className="moderation-inline-failure" role="alert">
                        <CircleAlert aria-hidden="true" />
                        <span>
                          {t('moderation.report.failure', { code: result.error.code })}
                          {result.error.retryAfterSeconds === undefined
                            ? ''
                            : ` ${t('moderation.report.retryAfter', {
                                count: result.error.retryAfterSeconds,
                              })}`}
                        </span>
                      </div>
                    ) : null}
                    <footer className="moderation-dialog-actions">
                      <Button disabled={mutation.isPending} onClick={close} tone="quiet">
                        {t('moderation.report.cancel')}
                      </Button>
                      <Button
                        disabled={mutation.isPending}
                        icon={
                          mutation.isPending ? (
                            <LoaderCircle aria-hidden="true" />
                          ) : (
                            <Flag aria-hidden="true" />
                          )
                        }
                        tone="primary"
                        type="submit"
                      >
                        {t(
                          mutation.isPending
                            ? 'moderation.report.submitting'
                            : 'moderation.report.submit',
                        )}
                      </Button>
                    </footer>
                  </form>
                )}
              </motion.section>
            </motion.div>
          ) : null}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}

function parseReason(value: string): ModerationReason {
  return moderationReasons.find((reason) => reason === value) ?? 'other';
}
