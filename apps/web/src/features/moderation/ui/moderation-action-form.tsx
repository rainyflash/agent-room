import { Button } from '@agent-room/ui-system';
import { CircleAlert, LoaderCircle, ShieldAlert } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  moderationActionKinds,
  moderationReasons,
  type ApplyModerationActionInput,
  type ModerationActionKind,
  type ModerationCase,
  type ModerationReason,
} from '@/features/moderation/domain/moderation';

export type ModerationActionFormProps = {
  readonly cases: readonly ModerationCase[];
  readonly onApply: (input: ApplyModerationActionInput) => void;
  readonly onReauthenticate: () => void;
  readonly pending: boolean;
  readonly recentlyAuthenticated: boolean;
};

export function ModerationActionForm({
  cases,
  onApply,
  onReauthenticate,
  pending,
  recentlyAuthenticated,
}: ModerationActionFormProps) {
  const { t } = useTranslation();
  const actionableCases = cases.filter(
    (moderationCase) =>
      moderationCase.targetKind === 'event' || moderationCase.targetKind === 'principal',
  );
  const [kind, setKind] = useState<ModerationActionKind>('hide');
  const [reason, setReason] = useState<ModerationReason>('other');
  const [targetReference, setTargetReference] = useState('');
  const [caseId, setCaseId] = useState('');
  const [durationSeconds, setDurationSeconds] = useState(0);
  const [acknowledged, setAcknowledged] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const valid = targetReference.trim().length > 0 && acknowledged && recentlyAuthenticated;

  const selectCase = (selectedCaseId: string): void => {
    setCaseId(selectedCaseId);
    const selected = actionableCases.find(
      (moderationCase) => moderationCase.caseId === selectedCaseId,
    );
    if (selected === undefined) {
      return;
    }
    setKind(selected.targetKind === 'event' ? 'hide' : 'mute');
    setReason(selected.reason);
    setTargetReference(selected.targetReference);
  };

  return (
    <form
      className="moderation-action-form"
      onSubmit={(event) => {
        event.preventDefault();
        setAttempted(true);
        if (!valid) {
          return;
        }
        onApply({
          ...(caseId === '' ? {} : { caseId }),
          ...(kind === 'mute' && durationSeconds > 0
            ? { expiresAtUnixMs: Date.now() + durationSeconds * 1_000 }
            : {}),
          impactAcknowledged: true,
          kind,
          reason,
          targetKind: kind === 'hide' ? 'event' : 'principal',
          targetReference: targetReference.trim(),
        });
      }}
    >
      <header>
        <ShieldAlert aria-hidden="true" />
        <div>
          <h3>{t('moderation.governance.action.new')}</h3>
          <p>{t('moderation.governance.detail')}</p>
        </div>
      </header>
      {actionableCases.length === 0 ? null : (
        <label>
          <span>{t('moderation.governance.action.case')}</span>
          <select
            disabled={pending}
            onChange={(event) => {
              selectCase(event.currentTarget.value);
            }}
            value={caseId}
          >
            <option value="">—</option>
            {actionableCases.map((moderationCase) => (
              <option key={moderationCase.caseId} value={moderationCase.caseId}>
                {t(`moderation.reason.${moderationCase.reason}`)} · {moderationCase.caseId}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="moderation-action-form__grid">
        <label>
          <span>{t('moderation.governance.action.kind')}</span>
          <select
            disabled={pending}
            onChange={(event) => {
              setKind(parseActionKind(event.currentTarget.value));
              setCaseId('');
              setTargetReference('');
            }}
            value={kind}
          >
            {moderationActionKinds.map((option) => (
              <option key={option} value={option}>
                {t(`moderation.kind.${option}`)}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>{t('moderation.governance.action.reason')}</span>
          <select
            disabled={pending}
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
      </div>
      <label>
        <span>{t('moderation.governance.action.target')}</span>
        <input
          disabled={pending}
          maxLength={1_024}
          onChange={(event) => {
            setTargetReference(event.currentTarget.value);
          }}
          placeholder={t(
            kind === 'hide'
              ? 'moderation.governance.action.targetEvent'
              : 'moderation.governance.action.targetPrincipal',
          )}
          value={targetReference}
        />
      </label>
      {kind === 'mute' ? (
        <label>
          <span>{t('moderation.governance.action.duration')}</span>
          <select
            disabled={pending}
            onChange={(event) => {
              setDurationSeconds(Number(event.currentTarget.value));
            }}
            value={durationSeconds}
          >
            <option value={0}>{t('moderation.governance.action.indefinite')}</option>
            <option value={3_600}>{t('moderation.governance.action.hour')}</option>
            <option value={86_400}>{t('moderation.governance.action.day')}</option>
            <option value={604_800}>{t('moderation.governance.action.week')}</option>
          </select>
        </label>
      ) : null}
      <label className="moderation-evidence-choice">
        <input
          checked={acknowledged}
          disabled={pending}
          onChange={(event) => {
            setAcknowledged(event.currentTarget.checked);
          }}
          type="checkbox"
        />
        <span>
          <strong>{t('moderation.governance.action.acknowledge')}</strong>
        </span>
      </label>
      {!recentlyAuthenticated ? (
        <div className="moderation-inline-failure" role="note">
          <CircleAlert aria-hidden="true" />
          <span>{t('moderation.governance.action.recentAuth')}</span>
          <Button onClick={onReauthenticate} size="compact" tone="quiet">
            {t('moderation.governance.action.reauthenticate')}
          </Button>
        </div>
      ) : null}
      {attempted && !valid ? (
        <p className="moderation-form-validation" role="alert">
          {t('moderation.governance.action.invalid')}
        </p>
      ) : null}
      <Button
        disabled={pending || !recentlyAuthenticated}
        icon={pending ? <LoaderCircle aria-hidden="true" /> : <ShieldAlert aria-hidden="true" />}
        tone="primary"
        type="submit"
      >
        {t(
          pending ? 'moderation.governance.action.applying' : 'moderation.governance.action.apply',
        )}
      </Button>
    </form>
  );
}

function parseActionKind(value: string): ModerationActionKind {
  return moderationActionKinds.find((kind) => kind === value) ?? 'hide';
}

function parseReason(value: string): ModerationReason {
  return moderationReasons.find((reason) => reason === value) ?? 'other';
}
