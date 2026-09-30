import { Banner, Button, Details, Dialog, Field, Spinner } from '@agent-room/ui-system';
import { Flag, ShieldCheck } from 'lucide-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  moderationCaseListQueryKey,
  moderationRoomCaseListQueryKey,
} from '@/features/moderation/data/moderation-queries';
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

/**
 * 举报一条消息或资料：选原因、写几句说明，愿意的话附上看得到的摘要。只附带消息的标识，
 * 不读取、不上传解密后的全文。
 */
export function MessageReportControl({ catalogId, gateway, message }: MessageReportControlProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const formId = useId();
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
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: moderationCaseListQueryKey }),
          queryClient.invalidateQueries({ queryKey: moderationRoomCaseListQueryKey(catalogId) }),
        ]);
      }
    },
  });
  const result = mutation.data ?? null;

  const close = (): void => {
    if (!mutation.isPending) setOpen(false);
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
      <Button icon={<Flag aria-hidden="true" />} onClick={begin} size="compact" tone="ghost">
        {t('moderation.report.action')}
      </Button>
      {open ? (
        <Dialog
          className="moderation-report"
          closeLabel={t('moderation.report.close')}
          description={t('moderation.report.detail')}
          footer={
            result?.ok === true ? (
              <Button onClick={close}>{t('moderation.report.done')}</Button>
            ) : (
              <>
                <Button disabled={mutation.isPending} onClick={close} tone="quiet">
                  {t('moderation.report.cancel')}
                </Button>
                <Button
                  disabled={mutation.isPending}
                  form={formId}
                  icon={mutation.isPending ? <Spinner /> : <Flag aria-hidden="true" />}
                  type="submit"
                >
                  {t(
                    mutation.isPending
                      ? 'moderation.report.submitting'
                      : 'moderation.report.submit',
                  )}
                </Button>
              </>
            )
          }
          icon={<Flag />}
          onClose={close}
          title={t('moderation.report.title')}
        >
          {result?.ok === true ? (
            <Banner
              className="moderation-report-success"
              title={t('moderation.report.success')}
              tone="success"
            >
              <p>{t('moderation.report.successDetail')}</p>
              <Details summary={t('moderation.report.details')}>
                <code>{result.value.caseId}</code>
              </Details>
            </Banner>
          ) : (
            <form
              className="moderation-report-form"
              id={formId}
              onSubmit={(event) => {
                event.preventDefault();
                mutation.mutate();
              }}
            >
              <Field label={t('moderation.report.reason')}>
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
              </Field>
              <Field label={t('moderation.report.description')}>
                <textarea
                  disabled={mutation.isPending}
                  maxLength={4_096}
                  onChange={(event) => {
                    setDescription(event.currentTarget.value);
                  }}
                  placeholder={t('moderation.report.descriptionPlaceholder')}
                  rows={4}
                  value={description}
                />
              </Field>
              <label className="moderation-evidence-choice">
                <input
                  checked={includePreview}
                  disabled={mutation.isPending || message.preview === null}
                  onChange={(event) => {
                    setIncludePreview(event.currentTarget.checked);
                  }}
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
                <Banner tone="danger" title={t('moderation.report.failed')}>
                  {result.error.retryAfterSeconds === undefined ? null : (
                    <p>
                      {t('moderation.report.retryAfter', {
                        count: result.error.retryAfterSeconds,
                      })}
                    </p>
                  )}
                  <Details summary={t('moderation.report.details')}>
                    <code>{result.error.code}</code>
                  </Details>
                </Banner>
              ) : null}
            </form>
          )}
        </Dialog>
      ) : null}
    </>
  );
}

function parseReason(value: string): ModerationReason {
  return moderationReasons.find((reason) => reason === value) ?? 'other';
}
