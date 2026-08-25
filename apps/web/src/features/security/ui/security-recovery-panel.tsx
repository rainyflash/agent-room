import { Button } from '@agent-room/ui-system';
import { useMutation } from '@tanstack/react-query';
import { Check, Copy, KeyRound, LoaderCircle, RotateCcw } from 'lucide-react';
import { useId, useState, type FormEvent } from 'react';
import { useTranslation } from 'react-i18next';

import type {
  MatrixRecoveryProgress,
  MatrixSecurityFailure,
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
import { isValidRecoveryPassphrase } from '@/features/security/domain/matrix-security';
import { recoveryMessageKey } from '@/features/security/ui/security-copy';
import { SecurityFailureNotice } from '@/features/security/ui/security-failure-notice';

export type SecurityRecoveryPanelProps = {
  readonly gateway: MatrixSecurityGateway;
  readonly onChanged: () => void;
  readonly snapshot: MatrixSecuritySnapshot;
};

type RecoveryMode = 'recover' | 'setup';

export function SecurityRecoveryPanel({
  gateway,
  onChanged,
  snapshot,
}: SecurityRecoveryPanelProps) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<RecoveryMode | null>(null);
  const [passphrase, setPassphrase] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [credential, setCredential] = useState('');
  const [progress, setProgress] = useState<MatrixRecoveryProgress | null>(null);
  const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const setup = useMutation({
    mutationFn: async () => await gateway.setupRecovery({ passphrase }),
    onSuccess: (result) => {
      if (!result.ok) {
        return;
      }
      setRecoveryKey(result.value.recoveryKey);
      setPassphrase('');
      setConfirmation('');
      onChanged();
    },
  });
  const recover = useMutation({
    mutationFn: async () => await gateway.recover({ credential }, setProgress),
    onSuccess: (result) => {
      if (!result.ok) {
        return;
      }
      setCredential('');
      onChanged();
    },
  });

  const switchMode = (next: RecoveryMode): void => {
    setup.reset();
    recover.reset();
    setProgress(null);
    setMode(next);
  };
  const cancel = (): void => {
    setup.reset();
    recover.reset();
    setPassphrase('');
    setConfirmation('');
    setCredential('');
    setProgress(null);
    setMode(null);
  };

  return (
    <section
      aria-labelledby="security-recovery-title"
      className="security-recovery"
      id="security-recovery"
    >
      <header className="security-section-heading security-recovery__heading">
        <div>
          <h2 id="security-recovery-title">{t('security.recovery.title')}</h2>
          <p>{t('security.recovery.detail')}</p>
        </div>
        <span className={`security-recovery__status security-recovery__status--${snapshot.backup}`}>
          {t(recoveryMessageKey[snapshot.backup])}
        </span>
      </header>

      {recoveryKey !== null ? (
        <RecoveryKeyReceipt
          copied={copied}
          onAcknowledge={() => {
            setRecoveryKey(null);
            setCopied(false);
            setMode(null);
            setup.reset();
          }}
          onCopy={async () => {
            try {
              await navigator.clipboard.writeText(recoveryKey);
              setCopied(true);
            } catch {
              setCopied(false);
            }
          }}
          recoveryKey={recoveryKey}
        />
      ) : mode === 'setup' ? (
        <RecoverySetupForm
          confirmation={confirmation}
          failure={setup.data?.ok === false ? setup.data.error : null}
          onCancel={cancel}
          onConfirmationChange={setConfirmation}
          onPassphraseChange={setPassphrase}
          onSubmit={() => setup.mutate()}
          passphrase={passphrase}
          pending={setup.isPending}
        />
      ) : mode === 'recover' ? (
        <RecoveryUnlockForm
          credential={credential}
          failure={recover.data?.ok === false ? recover.data.error : null}
          onCancel={cancel}
          onCredentialChange={setCredential}
          onSubmit={() => {
            setProgress({ stage: 'fetching' });
            recover.mutate();
          }}
          pending={recover.isPending}
          progress={progress}
          result={recover.data?.ok === true ? recover.data.value : null}
        />
      ) : (
        <div className="security-recovery__actions">
          <Button
            icon={<KeyRound aria-hidden="true" />}
            onClick={() => switchMode('setup')}
            tone={snapshot.backup === 'missing' ? 'primary' : 'ghost'}
          >
            {t('security.recovery.setup')}
          </Button>
          <Button
            icon={<RotateCcw aria-hidden="true" />}
            onClick={() => switchMode('recover')}
            tone={
              snapshot.backup === 'locked' || snapshot.backup === 'untrusted' ? 'primary' : 'quiet'
            }
          >
            {t('security.recovery.recover')}
          </Button>
        </div>
      )}
    </section>
  );
}

type RecoverySetupFormProps = {
  readonly confirmation: string;
  readonly failure: MatrixSecurityFailure | null;
  readonly onCancel: () => void;
  readonly onConfirmationChange: (value: string) => void;
  readonly onPassphraseChange: (value: string) => void;
  readonly onSubmit: () => void;
  readonly passphrase: string;
  readonly pending: boolean;
};

function RecoverySetupForm({
  confirmation,
  failure,
  onCancel,
  onConfirmationChange,
  onPassphraseChange,
  onSubmit,
  passphrase,
  pending,
}: RecoverySetupFormProps) {
  const { t } = useTranslation();
  const passphraseId = useId();
  const confirmationId = useId();
  const valid = isValidRecoveryPassphrase(passphrase) && passphrase === confirmation;
  const mismatch = confirmation.length > 0 && passphrase !== confirmation;
  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (valid) {
      onSubmit();
    }
  };

  return (
    <form className="security-recovery-form" onSubmit={submit}>
      <div className="security-recovery-form__intro">
        <h3>{t('security.recovery.setupTitle')}</h3>
        <p>{t('security.recovery.setupDetail')}</p>
      </div>
      <label htmlFor={passphraseId}>
        <span>{t('security.recovery.passphrase')}</span>
        <input
          aria-label={t('security.recovery.passphrase')}
          autoComplete="new-password"
          disabled={pending}
          id={passphraseId}
          maxLength={256}
          minLength={12}
          onChange={(event) => onPassphraseChange(event.target.value)}
          required
          type="password"
          value={passphrase}
        />
        <small>{t('security.recovery.passphraseHint')}</small>
      </label>
      <label htmlFor={confirmationId}>
        <span>{t('security.recovery.confirmPassphrase')}</span>
        <input
          aria-invalid={mismatch}
          aria-label={t('security.recovery.confirmPassphrase')}
          autoComplete="new-password"
          disabled={pending}
          id={confirmationId}
          maxLength={256}
          minLength={12}
          onChange={(event) => onConfirmationChange(event.target.value)}
          required
          type="password"
          value={confirmation}
        />
        {mismatch ? (
          <small className="is-error">{t('security.recovery.passphraseMismatch')}</small>
        ) : null}
      </label>
      {failure === null ? null : <SecurityFailureNotice failure={failure} />}
      <div className="security-recovery-form__actions">
        <Button disabled={pending} onClick={onCancel} tone="quiet">
          {t('security.recovery.cancel')}
        </Button>
        <Button
          disabled={!valid || pending}
          icon={
            pending ? (
              <LoaderCircle aria-hidden="true" className="security-spin" />
            ) : (
              <KeyRound aria-hidden="true" />
            )
          }
          type="submit"
        >
          {t(pending ? 'security.recovery.creating' : 'security.recovery.create')}
        </Button>
      </div>
    </form>
  );
}

type RecoveryUnlockFormProps = {
  readonly credential: string;
  readonly failure: MatrixSecurityFailure | null;
  readonly onCancel: () => void;
  readonly onCredentialChange: (value: string) => void;
  readonly onSubmit: () => void;
  readonly pending: boolean;
  readonly progress: MatrixRecoveryProgress | null;
  readonly result: { readonly imported: number; readonly total: number } | null;
};

function RecoveryUnlockForm({
  credential,
  failure,
  onCancel,
  onCredentialChange,
  onSubmit,
  pending,
  progress,
  result,
}: RecoveryUnlockFormProps) {
  const { t } = useTranslation();
  const credentialId = useId();
  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (credential.length > 0) {
      onSubmit();
    }
  };

  return (
    <form className="security-recovery-form" onSubmit={submit}>
      <div className="security-recovery-form__intro">
        <h3>{t('security.recovery.recoverTitle')}</h3>
        <p>{t('security.recovery.recoverDetail')}</p>
      </div>
      <label className="security-recovery-form__credential" htmlFor={credentialId}>
        <span>{t('security.recovery.credential')}</span>
        <textarea
          aria-label={t('security.recovery.credential')}
          autoComplete="current-password"
          disabled={pending}
          id={credentialId}
          maxLength={1_024}
          onChange={(event) => onCredentialChange(event.target.value)}
          required
          rows={3}
          value={credential}
        />
      </label>
      {pending && progress !== null ? (
        <p aria-live="polite" className="security-recovery__progress" role="status">
          <LoaderCircle aria-hidden="true" className="security-spin" />
          <span>
            {progress.stage === 'fetching'
              ? t('security.recovery.fetching')
              : t('security.recovery.importing', progress)}
          </span>
        </p>
      ) : null}
      {result === null ? null : (
        <p aria-live="polite" className="security-recovery__complete" role="status">
          <Check aria-hidden="true" />
          <span>{t('security.recovery.complete', result)}</span>
        </p>
      )}
      {failure === null ? null : <SecurityFailureNotice failure={failure} />}
      <div className="security-recovery-form__actions">
        <Button disabled={pending} onClick={onCancel} tone="quiet">
          {t('security.recovery.cancel')}
        </Button>
        <Button
          disabled={credential.length === 0 || pending}
          icon={
            pending ? (
              <LoaderCircle aria-hidden="true" className="security-spin" />
            ) : (
              <RotateCcw aria-hidden="true" />
            )
          }
          type="submit"
        >
          {t(pending ? 'security.recovery.restoring' : 'security.recovery.restore')}
        </Button>
      </div>
    </form>
  );
}

type RecoveryKeyReceiptProps = {
  readonly copied: boolean;
  readonly onAcknowledge: () => void;
  readonly onCopy: () => Promise<void>;
  readonly recoveryKey: string;
};

function RecoveryKeyReceipt({
  copied,
  onAcknowledge,
  onCopy,
  recoveryKey,
}: RecoveryKeyReceiptProps) {
  const { t } = useTranslation();
  return (
    <div className="security-recovery-key">
      <div>
        <h3>{t('security.recovery.keyTitle')}</h3>
        <p>{t('security.recovery.keyDetail')}</p>
      </div>
      <output>{recoveryKey}</output>
      <div className="security-recovery-form__actions">
        <Button icon={<Copy aria-hidden="true" />} onClick={() => void onCopy()} tone="ghost">
          {t(copied ? 'security.recovery.copied' : 'security.recovery.copy')}
        </Button>
        <Button icon={<Check aria-hidden="true" />} onClick={onAcknowledge} tone="primary">
          {t('security.recovery.saved')}
        </Button>
      </div>
    </div>
  );
}
