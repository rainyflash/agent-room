import { Button } from '@agent-room/ui-system';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type {
  AgentRecoveryGateway,
  AgentRecoveryFailure,
} from '@/features/security/domain/agent-recovery';
import { agentRecoveryRequestSchema } from '@/features/security/domain/agent-recovery';
import { err, ok } from '@/shared/result';

export function AgentRecoveryPanel({ gateway }: { readonly gateway: AgentRecoveryGateway }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const id = useId();
  const [selected, setSelected] = useState('');
  const [mode, setMode] = useState<'enable' | 'restore' | null>(null);
  const [secret, setSecret] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
  const sessions = useQuery({
    queryKey: ['local-agent-recovery-sessions'],
    queryFn: () => gateway.sessions(),
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  const entries = sessions.data?.ok ? sessions.data.value : [];
  const sessionId = selected || (entries.find((entry) => entry.state === 'ready')?.sessionId ?? '');
  const stateKey = ['local-agent-recovery-state', sessionId];
  const inspection = useQuery({
    queryKey: stateKey,
    enabled: sessionId.length > 0,
    queryFn: async () => {
      const result = await gateway.execute(sessionId, { action: 'inspect' });
      return result.ok ? ok(result.value.state) : result;
    },
  });
  const change = useMutation({
    mutationFn: async () => {
      if (mode === null) return err({ code: 'bridge.security.invalid_request', retryable: false });
      const result = await gateway.execute(
        sessionId,
        mode === 'enable'
          ? { action: mode, passphrase: secret }
          : { action: mode, credential: secret },
      );
      setSecret('');
      setConfirmation('');
      if (!result.ok) return result;
      setRecoveryKey(result.value.recoveryKey);
      setMode(null);
      await queryClient.invalidateQueries({ queryKey: stateKey });
      // 恢复密钥只保留在当前视图，不进入 React Query 的 mutation 缓存。
      return ok(undefined);
    },
  });
  const state = inspection.data?.ok ? inspection.data.value : null;
  const busy = change.isPending || recoveryKey !== null;
  const canSetup =
    state !== null && !state.recoveryAvailable && state.identity !== 'recovery_required';
  const canSubmit =
    mode !== null &&
    agentRecoveryRequestSchema.safeParse(
      mode === 'enable'
        ? { action: mode, passphrase: secret }
        : { action: mode, credential: secret },
    ).success &&
    (mode !== 'enable' || secret === confirmation);
  const failure = !sessions.data?.ok
    ? sessions.data?.error
    : inspection.data?.ok === false
      ? inspection.data.error
      : change.data?.ok === false
        ? change.data.error
        : undefined;
  return (
    <section className="security-recovery" aria-labelledby={`${id}-title`}>
      <header className="security-section-heading security-recovery__heading">
        <div>
          <h2 id={`${id}-title`}>{t('security.agentRecovery.title')}</h2>
          <p>{t('security.agentRecovery.detail')}</p>
        </div>
        <Button
          disabled={busy || mode !== null || sessions.isFetching}
          size="compact"
          onClick={() => {
            void sessions.refetch();
            if (sessionId.length > 0) void inspection.refetch();
          }}
        >
          {t('security.action.refresh')}
        </Button>
      </header>
      {sessions.isPending ? (
        <p role="status">{t('security.loading.title')}</p>
      ) : entries.length === 0 ? (
        sessions.data?.ok === true ? (
          <p>{t('security.agentRecovery.empty')}</p>
        ) : null
      ) : (
        <label className="security-agent-recovery-selection" htmlFor={`${id}-agent`}>
          <span>{t('security.agentRecovery.select')}</span>
          <select
            id={`${id}-agent`}
            value={sessionId}
            disabled={busy}
            onChange={(event) => {
              setSelected(event.target.value);
              setMode(null);
              setSecret('');
              setConfirmation('');
              change.reset();
            }}
          >
            <option value="" disabled>
              {t('security.agentRecovery.select')}
            </option>
            {entries.map((entry) => (
              <option
                key={entry.sessionId}
                value={entry.sessionId}
                disabled={entry.state !== 'ready'}
              >
                {entry.displayName}
              </option>
            ))}
          </select>
        </label>
      )}
      {state === null ? null : (
        <div className="security-account-line">
          <div>
            <span>{t('security.agentRecovery.identity')}</span>
            <strong>{state.userId}</strong>
          </div>
          <span role="status">
            {t(
              state.identity === 'ready' && state.backupEnabled && state.recoveryAvailable
                ? 'security.agentRecovery.ready'
                : state.recoveryAvailable
                  ? 'security.agentRecovery.locked'
                  : 'security.agentRecovery.missing',
            )}
          </span>
        </div>
      )}
      {recoveryKey !== null ? (
        <div className="security-recovery-key" role="status">
          <h3>{t('security.recovery.keyTitle')}</h3>
          <p>{t('security.recovery.keyDetail')}</p>
          <output aria-label={t('security.recovery.keyTitle')}>{recoveryKey}</output>
          <Button
            onClick={() => {
              setRecoveryKey(null);
              change.reset();
            }}
          >
            {t('security.recovery.saved')}
          </Button>
        </div>
      ) : mode !== null ? (
        <form
          className="security-recovery-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (canSubmit && !change.isPending) change.mutate();
          }}
        >
          <label htmlFor={`${id}-secret`}>
            <span>
              {t(
                mode === 'enable' ? 'security.recovery.passphrase' : 'security.recovery.credential',
              )}
            </span>
            <input
              id={`${id}-secret`}
              type="password"
              autoComplete={mode === 'enable' ? 'new-password' : 'off'}
              maxLength={1024}
              disabled={change.isPending}
              value={secret}
              onChange={(event) => {
                setSecret(event.target.value);
              }}
            />
          </label>
          {mode === 'enable' ? (
            <label htmlFor={`${id}-confirmation`}>
              <span>{t('security.recovery.confirmPassphrase')}</span>
              <input
                id={`${id}-confirmation`}
                type="password"
                autoComplete="new-password"
                maxLength={1024}
                disabled={change.isPending}
                value={confirmation}
                onChange={(event) => {
                  setConfirmation(event.target.value);
                }}
              />
            </label>
          ) : null}
          <p>
            {t(
              mode === 'enable'
                ? 'security.recovery.passphraseHint'
                : 'security.agentRecovery.restoreHint',
            )}
          </p>
          {mode === 'enable' && confirmation.length > 0 && secret !== confirmation ? (
            <p role="alert">{t('security.recovery.passphraseMismatch')}</p>
          ) : null}
          <div className="security-recovery-form__actions">
            <Button type="submit" disabled={!canSubmit || change.isPending}>
              {t(
                change.isPending
                  ? 'security.agentRecovery.working'
                  : mode === 'enable'
                    ? 'security.recovery.create'
                    : 'security.recovery.restore',
              )}
            </Button>
            <Button
              disabled={change.isPending}
              tone="quiet"
              onClick={() => {
                setMode(null);
                setSecret('');
                setConfirmation('');
                change.reset();
              }}
            >
              {t('security.recovery.cancel')}
            </Button>
          </div>
        </form>
      ) : state !== null ? (
        <div className="security-recovery__actions">
          {canSetup ? (
            <Button
              onClick={() => {
                setSelected(sessionId);
                setMode('enable');
                change.reset();
              }}
            >
              {t('security.recovery.setup')}
            </Button>
          ) : state.recoveryAvailable ? (
            <Button
              onClick={() => {
                setSelected(sessionId);
                setMode('restore');
                change.reset();
              }}
            >
              {t('security.agentRecovery.restore')}
            </Button>
          ) : (
            <p>{t('security.agentRecovery.unrecoverable')}</p>
          )}
        </div>
      ) : null}
      {failure === undefined ? null : <p role="alert">{t(recoveryFailureMessage(failure))}</p>}
      {change.data?.ok === true && recoveryKey === null ? (
        <p role="status">{t('security.agentRecovery.complete')}</p>
      ) : null}
    </section>
  );
}

function recoveryFailureMessage(failure: AgentRecoveryFailure) {
  switch (failure.code) {
    case 'bridge.security.recovery_rejected':
      return 'security.failure.recovery_key_rejected';
    case 'bridge.security.recovery_already_configured':
      return 'security.failure.recovery_already_configured';
    case 'bridge.security.recovery_unavailable':
      return 'security.failure.recovery_key_missing';
    case 'bridge.security.invalid_request':
      return 'security.failure.recovery_credential_invalid';
    default:
      return 'security.agentRecovery.failed';
  }
}
