import { Button } from '@agent-room/ui-system';
import { RefreshCw } from 'lucide-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useRef, useState } from 'react';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import { enableReception, RECEPTION_AUTHORIZATION_DAYS } from '../application/enable-reception';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { useSession } from '@/features/session/ui/session-provider';
import { useAutomationGrantList } from '@/features/automation/data/automation-grant-queries';
import type { AutomationGrant } from '@/features/automation/domain/automation-grant';
import type { HostSessionDiagnostics } from '../domain/desktop-runtime';
import {
  receptionFailureAdvice,
  receptionGrants,
  type ReceiverAction,
  type ReceiverView,
} from '../domain/reception';
import { err, type Result } from '@/shared/result';
import type { DesktopRuntimeFailure } from '../domain/desktop-runtime';
import { receiverAgentId, receiverStatus } from '../domain/reception-status';
import { useReceptionQueries } from './use-reception-queries';
import './reception-panel.css';
import { ReceptionOwnershipPanel, receptionOwnershipQueryKeys } from './reception-ownership-panel';

const unavailable = () => err({ code: 'receiver.unavailable', retryable: false });

export function ReceptionPanel({
  agentId,
  roomId,
}: {
  readonly agentId?: string;
  readonly roomId?: string;
}) {
  const { t } = useTranslation();
  const { localRuntime: gateway, automation } = useAppServices();
  const { snapshot } = useSession();
  const principal = snapshot.context.principal;
  const queryClient = useQueryClient();
  const { queryKey, receivers, sessions } = useReceptionQueries(
    gateway,
    principal?.principalId ?? null,
  );
  const grantsQuery = useAutomationGrantList(automation);
  const grants = grantsQuery.data?.ok ? grantsQuery.data.value : [];
  const allViews = receivers.data?.ok
    ? receivers.data.value.filter(
        (view) => view.state.binding.policy.allowedPrincipalId === principal?.principalId,
      )
    : [];
  const views = allViews.filter(
    (view) =>
      (agentId === undefined ||
        receiverAgentId(view, sessions.data?.ok ? sessions.data.value : [], grants) === agentId) &&
      (roomId === undefined || view.state.binding.policy.roomId === roomId),
  );
  const offers = sessions.data?.ok
    ? sessions.data.value.filter(
        (session) =>
          session.receptionOffer &&
          (agentId === undefined || session.session.agentId === agentId) &&
          (roomId === undefined || session.receptionOffer.roomId === roomId) &&
          !allViews.some((view) => view.state.binding.session.sessionKey === session.sessionKey),
      )
    : [];
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'copyFailed'>('idle');
  const pendingGrantIds = useRef(new Map<string, string>());
  const command = useMutation({
    mutationFn: (run: () => Promise<Result<void, DesktopRuntimeFailure>>) => run(),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey });
      await grantsQuery.refetch();
    },
  });
  const act = (id: string, request: ReceiverAction) => {
    command.mutate(() => gateway.receiverAction?.(id, request) ?? Promise.resolve(unavailable()));
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(t('reception.prompt'));
      setCopyState('copied');
    } catch {
      setCopyState('copyFailed');
    }
  };
  const failure = [command.data, receivers.data, sessions.data, grantsQuery.data].find(
    (result) => result?.ok === false,
  );
  return (
    <section className="reception-panel" aria-label={t('reception.title')}>
      <div className="reception-section-head">
        <h3>{t('reception.title')}</h3>
        <button
          type="button"
          className="ar-icon-button reception-refresh"
          aria-label={t('reception.refresh')}
          title={t('reception.refresh')}
          onClick={() => {
            const principalId = principal?.principalId ?? null;
            void queryClient.invalidateQueries({ queryKey });
            void queryClient.invalidateQueries({
              queryKey: receptionOwnershipQueryKeys.records(principalId),
            });
            void queryClient.invalidateQueries({
              queryKey: receptionOwnershipQueryKeys.devices(principalId),
            });
            void grantsQuery.refetch();
          }}
        >
          <RefreshCw aria-hidden="true" />
        </button>
      </div>
      <p className="reception-panel__intro">
        {t(agentId === undefined ? 'reception.description' : 'reception.agentDescription')}
      </p>
      <div className="reception-actions">
        <Button size="compact" tone="ghost" onClick={() => void copy()}>
          {t(copyState === 'copied' ? 'reception.copied' : 'reception.copy')}
        </Button>
      </div>
      {copyState === 'copyFailed' ? <p role="status">{t('reception.copyFailed')}</p> : null}
      <details className="ar-disclosure">
        <summary>{t('reception.showPrompt')}</summary>
        <div className="ar-disclosure__body">
          <p className="reception-copy">{t('reception.prompt')}</p>
        </div>
      </details>
      {receivers.isPending ? <p>{t('reception.loading')}</p> : null}
      {failure?.ok === false ? (
        <p role="alert">
          {t('reception.failed')} <code>{failure.error.code}</code>
        </p>
      ) : null}
      {receivers.isError || sessions.isError || grantsQuery.isError || command.isError ? (
        <p role="alert">{t('reception.failed')}</p>
      ) : null}
      {!principal ? <p>{t('reception.login')}</p> : null}
      {views.length === 0 && offers.length === 0 && !receivers.isPending ? (
        <p>{t(agentId === undefined ? 'reception.empty' : 'reception.agentUnavailable')}</p>
      ) : null}
      {offers.map((session) => (
        <ReceptionOffer
          key={session.session.sessionId}
          session={session}
          grants={grants}
          busy={command.isPending || !principal}
          onEnable={() => {
            if (principal === null || !grantsQuery.data?.ok) return;
            const key = session.session.sessionId;
            const grantId = pendingGrantIds.current.get(key) ?? new BrowserUuidV7Factory().next();
            pendingGrantIds.current.set(key, grantId);
            command.mutate(() =>
              enableReception(
                {
                  session,
                  principalId: principal.principalId,
                  grantId,
                  grants,
                  now: Date.now(),
                },
                { automation, runtime: gateway },
              ),
            );
          }}
          canEnable={
            grantsQuery.data?.ok === true &&
            session.session.state === 'ready' &&
            session.receptionOffer?.roomCatalogId != null
          }
          onConfigure={(grantId, executable) => {
            if (principal)
              command.mutate(
                () =>
                  gateway.configureReceiver?.({
                    sessionId: session.session.sessionId,
                    principalId: principal.principalId,
                    automationGrantId: grantId,
                    executable: executable || null,
                  }) ?? Promise.resolve(unavailable()),
              );
          }}
        />
      ))}
      {views.map((view) => (
        <ReceptionCard
          key={view.state.binding.host.taskId}
          view={view}
          grants={grants}
          busy={command.isPending}
          onAction={(request) => {
            act(view.state.binding.host.taskId, request);
          }}
        />
      ))}
      <ReceptionOwnershipPanel agentId={agentId} roomId={roomId} embedded />
    </section>
  );
}

function GrantSelect({
  grants,
  value,
  onChange,
}: {
  readonly grants: readonly AutomationGrant[];
  readonly value: string;
  readonly onChange: (value: string) => void;
}) {
  const { t, i18n } = useTranslation();
  return (
    <label>
      {t('reception.grant')}
      <select
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        <option value="">{t('reception.choose')}</option>
        {grants.map((grant) => (
          <option key={grant.grantId} value={grant.grantId}>
            {t('reception.grantLabel', {
              time: new Date(grant.expiresAtUnixMs).toLocaleString(i18n.language),
              limit: grant.maxMessagesPerMinute,
            })}
          </option>
        ))}
      </select>
    </label>
  );
}

function ReceptionOffer({
  session,
  grants,
  busy,
  onConfigure,
  onEnable,
  canEnable,
}: {
  readonly session: HostSessionDiagnostics;
  readonly grants: readonly AutomationGrant[];
  readonly busy: boolean;
  readonly onConfigure: (grantId: string, executable: string) => void;
  readonly onEnable: () => void;
  readonly canEnable: boolean;
}) {
  const { t } = useTranslation();
  const [grantId, setGrantId] = useState('');
  const [executable, setExecutable] = useState('');
  const offer = session.receptionOffer;
  if (!offer) return null;
  const allowed = receptionGrants(
    grants,
    session.session.agentId,
    offer.roomCatalogId,
    offer.instanceId,
    Date.now(),
  );
  return (
    <div className="reception-card">
      <strong className="reception-card__name">{session.displayName}</strong>
      <p className="reception-card__path">{offer.task.workspace}</p>
      <p className="reception-card__hint">
        {t('reception.enableDescription', { days: RECEPTION_AUTHORIZATION_DAYS })}
      </p>
      <div className="reception-actions">
        <Button size="compact" disabled={busy || !canEnable} onClick={onEnable}>
          {t('reception.enable')}
        </Button>
      </div>
      <details className="ar-disclosure">
        <summary>{t('reception.manual')}</summary>
        <div className="ar-disclosure__body">
          <GrantSelect grants={allowed} value={grantId} onChange={setGrantId} />
          {allowed.length === 0 ? (
            <p className="reception-card__hint">{t('reception.noGrant')}</p>
          ) : null}
          <details className="ar-disclosure ar-disclosure--nested">
            <summary>{t('reception.settings')}</summary>
            <div className="ar-disclosure__body">
              <label>
                {t('reception.executable')}
                <input
                  value={executable}
                  onChange={(event) => {
                    setExecutable(event.target.value);
                  }}
                />
              </label>
            </div>
          </details>
          <div className="reception-actions">
            <Button
              size="compact"
              disabled={busy || !allowed.some((grant) => grant.grantId === grantId)}
              onClick={() => {
                onConfigure(grantId, executable);
              }}
            >
              {t('reception.bind')}
            </Button>
          </div>
        </div>
      </details>
    </div>
  );
}

export function ReceptionCard({
  view,
  grants,
  busy,
  onAction,
}: {
  readonly view: ReceiverView;
  readonly grants: readonly AutomationGrant[];
  readonly busy: boolean;
  readonly onAction: (request: ReceiverAction) => void;
}) {
  const { t } = useTranslation();
  const { state, running } = view;
  const { binding, lastDelivery, checkpoint } = state;
  const [grantId, setGrantId] = useState(binding.automationGrantId);
  const [executable, setExecutable] = useState(binding.host.executable);
  const [workspace, setWorkspace] = useState(binding.host.workspace);
  const existingGrant = grants.find((grant) => grant.grantId === binding.automationGrantId);
  const allowed = receptionGrants(
    grants,
    state.agentId ?? existingGrant?.agentId ?? null,
    state.roomCatalogId,
    state.instanceId,
    Date.now(),
  );
  const pending = checkpoint.state === 'pending';
  const error = view.failure ?? lastDelivery?.failure;
  const status = receiverStatus(view);
  return (
    <article className="reception-card" data-status={status}>
      <strong className="reception-card__name">{binding.session.displayName}</strong>
      <div className="reception-card__facts">
        <span className="reception-chip" data-tone={status} role="status">
          {t(`reception.${status}`)}
        </span>
        <span className="reception-card__meta">
          {binding.host.hostType === 'claude_code' ? 'Claude Code' : 'Codex'}
        </span>
      </div>
      <p className="reception-card__path">{binding.host.workspace}</p>
      {error ? (
        <p className="reception-card__notice" role="alert">
          {t(`reception.failure.${receptionFailureAdvice(error.code)}`)} <code>{error.code}</code>
        </p>
      ) : null}
      {lastDelivery ? (
        <p className="reception-card__hint">{t(`reception.${lastDelivery.stage}`)}</p>
      ) : null}
      {pending ? <p className="reception-card__notice">{t('reception.pending')}</p> : null}
      {pending && !lastDelivery ? (
        <p className="reception-card__notice">{t('reception.legacyPending')}</p>
      ) : null}
      <div className="reception-actions">
        {running ? (
          <Button
            size="compact"
            disabled={busy}
            onClick={() => {
              onAction({ action: 'pause' });
            }}
          >
            {t(busy ? 'receptionOwner.stopping' : 'receptionOwner.manual')}
          </Button>
        ) : (
          <Button
            size="compact"
            disabled={
              busy ||
              (pending && !lastDelivery) ||
              (!pending && !allowed.some((grant) => grant.grantId === binding.automationGrantId))
            }
            onClick={() => {
              onAction({ action: pending ? 'verify' : 'start' });
            }}
          >
            {t(pending ? 'reception.verify' : 'receptionOwner.return')}
          </Button>
        )}
        {checkpoint.state === 'pending' && !running ? (
          <>
            <Button
              size="compact"
              tone="ghost"
              disabled={
                busy ||
                !lastDelivery ||
                !allowed.some((grant) => grant.grantId === binding.automationGrantId)
              }
              onClick={() => {
                onAction({ action: 'resolve', event: checkpoint.eventId, resolution: 'retry' });
              }}
            >
              {t('reception.retry')}
            </Button>
            <Button
              size="compact"
              tone="ghost"
              disabled={busy}
              onClick={() => {
                onAction({ action: 'resolve', event: checkpoint.eventId, resolution: 'skip' });
              }}
            >
              {t('reception.skip')}
            </Button>
          </>
        ) : null}
        <Button
          size="compact"
          tone="quiet"
          disabled={busy || pending || running}
          onClick={() => {
            onAction({ action: 'remove' });
          }}
        >
          {t('reception.remove')}
        </Button>
      </div>
      {!running ? (
        <details className="ar-disclosure">
          <summary>{t('reception.settings')}</summary>
          <div className="ar-disclosure__body">
            <GrantSelect grants={allowed} value={grantId} onChange={setGrantId} />
            {allowed.length === 0 ? (
              <p className="reception-card__hint">{t('reception.noGrant')}</p>
            ) : null}
            <label>
              {t('reception.executable')}
              <input
                value={executable}
                onChange={(event) => {
                  setExecutable(event.target.value);
                }}
              />
            </label>
            <label>
              {t('reception.workspace')}
              <input
                value={workspace}
                onChange={(event) => {
                  setWorkspace(event.target.value);
                }}
              />
            </label>
            <div className="reception-actions">
              <Button
                size="compact"
                tone="ghost"
                disabled={busy || !grantId || !executable || !workspace}
                onClick={() => {
                  onAction({ action: 'update', automationGrantId: grantId, executable, workspace });
                }}
              >
                {t('reception.update')}
              </Button>
            </div>
          </div>
        </details>
      ) : null}
    </article>
  );
}
