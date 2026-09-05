import { Button } from '@agent-room/ui-system';
import { CheckCircle2, Fingerprint, LoaderCircle, ShieldAlert, X } from 'lucide-react';
import { motion } from 'motion/react';
import { useCallback, useEffect, useRef, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import type {
  MatrixVerificationSession,
  MatrixVerificationSnapshot,
} from '@/features/security/domain/matrix-security';
import { failureMessageKey } from '@/features/security/ui/security-copy';

export type DeviceVerificationDialogProps = {
  readonly onClose: () => void;
  readonly onVerified: () => void;
  readonly session: MatrixVerificationSession;
  readonly targetName: string;
};

export function DeviceVerificationDialog({
  onClose,
  onVerified,
  session,
  targetName,
}: DeviceVerificationDialogProps) {
  const { t } = useTranslation();
  const subscribe = useCallback((listener: () => void) => session.subscribe(listener), [session]);
  const getSnapshot = useCallback(() => session.getSnapshot(), [session]);
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const verifiedReported = useRef(false);
  const terminal = isTerminal(snapshot);

  useEffect(() => {
    session.activate();
    return () => {
      session.deactivate();
    };
  }, [session]);

  useEffect(() => {
    if (snapshot.stage === 'verified' && !verifiedReported.current) {
      verifiedReported.current = true;
      onVerified();
    }
  }, [onVerified, snapshot.stage]);

  const close = useCallback((): void => {
    if (terminal) {
      onClose();
      return;
    }
    void session.cancel().finally(onClose);
  }, [onClose, session, terminal]);

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        close();
      }
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => {
      window.removeEventListener('keydown', closeOnEscape);
    };
  }, [close]);

  return createPortal(
    <motion.div
      animate={{ opacity: 1 }}
      className="security-dialog-overlay"
      initial={{ opacity: 0 }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          close();
        }
      }}
      transition={{ duration: 0.14 }}
    >
      <motion.aside
        animate={{ opacity: 1, scale: 1, y: 0 }}
        aria-labelledby="security-verification-title"
        aria-modal="true"
        className="security-dialog"
        initial={{ opacity: 0, scale: 0.98, y: 18 }}
        role="dialog"
        transition={{ damping: 30, stiffness: 340, type: 'spring' }}
      >
        <header className="security-dialog__header">
          <div>
            <Fingerprint aria-hidden="true" />
            <h2 id="security-verification-title">{t('security.verification.title')}</h2>
          </div>
          <button
            aria-label={t(terminal ? 'security.action.close' : 'security.verification.cancel')}
            className="security-icon-button"
            onClick={close}
            type="button"
          >
            <X aria-hidden="true" />
          </button>
        </header>
        <p className="security-dialog__target">
          {t('security.verification.target', { device: targetName })}
        </p>
        <VerificationBody session={session} snapshot={snapshot} />
      </motion.aside>
    </motion.div>,
    document.body,
  );
}

type VerificationBodyProps = {
  readonly session: MatrixVerificationSession;
  readonly snapshot: MatrixVerificationSnapshot;
};

function VerificationBody({ session, snapshot }: VerificationBodyProps) {
  const { t } = useTranslation();

  if (snapshot.stage === 'waiting') {
    return (
      <div aria-live="polite" className="security-dialog__state" role="status">
        <LoaderCircle aria-hidden="true" className="security-spin" />
        <h3>{t('security.verification.waiting')}</h3>
        <p>{t('security.verification.waitingDetail')}</p>
        {snapshot.transactionId === undefined ? null : (
          <code>
            {t('security.verification.transaction', { transaction: snapshot.transactionId })}
          </code>
        )}
        <Button onClick={() => void session.cancel()} tone="quiet">
          {t('security.verification.cancel')}
        </Button>
      </div>
    );
  }

  if (snapshot.stage === 'comparing') {
    return (
      <div aria-live="polite" className="security-dialog__comparison">
        <h3>{t('security.verification.comparing')}</h3>
        <p>{t('security.verification.comparingDetail')}</p>
        {snapshot.sas.emojis === undefined ? null : (
          <ol aria-label={t('security.verification.comparing')} className="security-sas-emojis">
            {snapshot.sas.emojis.map((emoji, index) => (
              <li key={`${emoji.symbol}-${index.toString()}`}>
                <span aria-hidden="true">{emoji.symbol}</span>
                <small>{emoji.label}</small>
              </li>
            ))}
          </ol>
        )}
        {snapshot.sas.decimals === undefined ? null : (
          <p className="security-sas-decimals">
            {t('security.verification.decimals', {
              values: snapshot.sas.decimals.join(' · '),
            })}
          </p>
        )}
        <div className="security-dialog__actions">
          <Button
            onClick={() => {
              session.mismatch();
            }}
            tone="alert"
          >
            {t('security.verification.mismatch')}
          </Button>
          <Button onClick={() => void session.confirm()} tone="primary">
            {t('security.verification.match')}
          </Button>
        </div>
      </div>
    );
  }

  if (snapshot.stage === 'confirming') {
    return (
      <div aria-live="polite" className="security-dialog__state" role="status">
        <LoaderCircle aria-hidden="true" className="security-spin" />
        <h3>{t('security.verification.confirming')}</h3>
        <p>{t('security.verification.confirmingDetail')}</p>
      </div>
    );
  }

  if (snapshot.stage === 'verified') {
    return (
      <div
        aria-live="polite"
        className="security-dialog__state security-dialog__state--success"
        role="status"
      >
        <CheckCircle2 aria-hidden="true" />
        <h3>{t('security.verification.verified')}</h3>
        <p>{t('security.verification.verifiedDetail')}</p>
      </div>
    );
  }

  return (
    <div
      aria-live="assertive"
      className="security-dialog__state security-dialog__state--failure"
      role="alert"
    >
      <ShieldAlert aria-hidden="true" />
      <h3>
        {t(
          snapshot.stage === 'cancelled'
            ? 'security.verification.cancelled'
            : 'security.verification.failed',
        )}
      </h3>
      {snapshot.stage === 'failed' ? <p>{t(failureMessageKey[snapshot.failure.code])}</p> : null}
    </div>
  );
}

function isTerminal(snapshot: MatrixVerificationSnapshot): boolean {
  return (
    snapshot.stage === 'cancelled' || snapshot.stage === 'failed' || snapshot.stage === 'verified'
  );
}
