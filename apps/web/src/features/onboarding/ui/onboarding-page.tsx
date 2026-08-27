import { Button, StatusMark } from '@agent-room/ui-system';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import {
  ArrowRight,
  Bot,
  Check,
  Download,
  ExternalLink,
  KeyRound,
  Network,
  PlugZap,
  RefreshCw,
  UserRound,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import type { PublicLobbyRouteTarget } from '@/features/lobby-entry/domain/public-lobby-entry';
import {
  projectOnboardingPhase,
  targetFor,
  targetMatches,
  type OnboardingPhase,
} from '@/features/onboarding/domain/onboarding';
import { useDesktopRuntime } from '@/features/desktop/ui/use-desktop-runtime';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { useSession } from '@/features/session/ui/session-provider';

import './onboarding-page.css';

const hostNames = {
  codex: 'Codex',
  'claude-code': 'Claude Code',
  cursor: 'Cursor',
} as const;

const phaseOrder: readonly OnboardingPhase[] = [
  'checking-account',
  'checking-agents',
  'runtime-required',
  'configuring-runtime',
  'authorizing-runtime',
  'ready',
];

export function OnboardingPage() {
  const { i18n, t } = useTranslation();
  const navigate = useNavigate();
  const reduceMotion = useReducedMotion();
  const { config, desktop, lobbyEntry, onboarding } = useAppServices();
  const { snapshot } = useSession();
  const sessionState = sessionStateName(snapshot.value);
  const principal = snapshot.context.principal;
  const accountReady = sessionState === 'ready' && principal !== null;
  const locale = principal?.locale ?? i18n.resolvedLanguage ?? 'en';
  const bootstrap = useQuery({
    enabled: accountReady,
    networkMode: 'always',
    queryFn: async () => await onboarding.bootstrap(locale),
    queryKey: ['onboarding', 'bootstrap', principal?.principalId ?? 'anonymous', locale] as const,
    retry: false,
    staleTime: 5_000,
  });
  const runtime = useDesktopRuntime(desktop);
  const resolved = bootstrap.data?.ok === true ? bootstrap.data.value : null;
  const expectedTarget =
    resolved === null ? null : targetFor(resolved.agent, resolved.lobby, locale);
  const runtimeMatches =
    expectedTarget !== null && targetMatches(runtime.snapshot?.agentTarget ?? null, expectedTarget);
  const bridgeSession = runtime.snapshot?.bridge.session ?? null;
  const runtimeReady =
    runtimeMatches &&
    runtime.snapshot?.bridge.lifecycle.phase === 'ready' &&
    bridgeSession?.agentId === resolved?.agent.agentId;
  const entry = useMutation({
    mutationFn: async (intent: LobbyEntryIntent) =>
      intent.kind === 'known'
        ? await lobbyEntry.enterKnown(intent.target)
        : await lobbyEntry.enter(intent.catalogId),
    onSuccess: (result) => {
      if (!result.ok) return;
      void navigate({
        params: {
          catalogId: result.value.catalogId,
          roomId: result.value.matrixRoomId,
        },
        search: {},
        to: '/lobby/$catalogId/instance/$roomId',
      });
    },
  });
  const phase = projectOnboardingPhase({
    accountReady,
    bootstrapFailed: bootstrap.data?.ok === false || bootstrap.isError,
    bootstrapReady: resolved !== null,
    bridgePhase: runtime.snapshot?.bridge.lifecycle.phase ?? null,
    desktopAvailable: runtime.available,
    runtimeSessionReady: runtimeReady,
    targetMatches: runtimeMatches,
  });
  const installedHosts = runtime.hosts.filter((host) => host.installed);
  const phaseIndex = Math.max(0, phaseOrder.indexOf(phase));

  return (
    <main className="onboarding" id="main-content">
      <header className="onboarding__topbar">
        <a aria-label={t('app.name')} className="onboarding__brand" href="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <div className="onboarding__operator">
          <span>{t(`onboarding.phase.${phase}`)}</span>
          <strong>{principal?.displayName ?? '—'}</strong>
        </div>
      </header>

      <section className="onboarding__layout">
        <motion.header
          animate={{ opacity: 1, y: 0 }}
          className="onboarding__intro"
          initial={{ opacity: 0, y: reduceMotion ? 0 : 14 }}
          transition={{ damping: 28, stiffness: 260, type: 'spring' }}
        >
          <p className="eyebrow">{t('onboarding.eyebrow')}</p>
          <h1>{t('onboarding.title')}</h1>
          <p>{t('onboarding.description')}</p>
          <ol aria-label={t('onboarding.title')} className="onboarding__progress">
            {(['account', 'agent', 'runtime'] as const).map((step, index) => (
              <li data-complete={phaseIndex > index ? 'true' : 'false'} key={step}>
                <span>{phaseIndex > index ? <Check aria-hidden="true" /> : `0${index + 1}`}</span>
                {t(`onboarding.${step}`)}
              </li>
            ))}
          </ol>
        </motion.header>

        <div className="onboarding__facts">
          <FactCard
            detail={t('onboarding.account.detail')}
            icon={<UserRound aria-hidden="true" />}
            status={accountReady ? 'complete' : 'pending'}
            title={t('onboarding.account')}
          >
            <strong>{principal?.displayName ?? t('connection.identity.pending')}</strong>
            <code>{principal?.matrixUserId ?? '—'}</code>
          </FactCard>

          <FactCard
            detail={
              resolved === null
                ? t('onboarding.phase.checking-agents')
                : t(
                    resolved.reusedExistingAgent
                      ? 'onboarding.agent.reused'
                      : 'onboarding.agent.created',
                  )
            }
            icon={<Bot aria-hidden="true" />}
            status={resolved === null ? 'pending' : 'complete'}
            title={t('onboarding.agent')}
          >
            <strong>{resolved?.agent.displayName ?? '—'}</strong>
            <code>{resolved?.agent.matrixUserId ?? '—'}</code>
          </FactCard>

          <FactCard
            detail={
              resolved === null
                ? t('onboarding.phase.checking-agents')
                : t('onboarding.lobby.detail', {
                    agents: resolved.lobby.onlineAgentCount,
                    instances: resolved.lobby.activeInstanceCount,
                  })
            }
            icon={<Network aria-hidden="true" />}
            status={resolved === null ? 'pending' : 'complete'}
            title={t('onboarding.lobby')}
          >
            <strong>{resolved?.lobby.name ?? '—'}</strong>
            <code>{resolved?.lobby.language ?? locale}</code>
          </FactCard>

          <FactCard
            detail={runtimeDetail(runtime.available, runtimeMatches, phase, {
              authorization: t('onboarding.runtime.authorization'),
              configured: t('onboarding.runtime.configured', {
                agent: resolved?.agent.displayName ?? 'Agent',
              }),
              configuring: t('onboarding.phase.configuring-runtime'),
              waiting: t('onboarding.runtime.waiting'),
              web: t('onboarding.runtime.web'),
            })}
            icon={<PlugZap aria-hidden="true" />}
            status={runtimeReady ? 'complete' : runtime.available ? 'active' : 'optional'}
            title={t('onboarding.runtime')}
          >
            <div className="onboarding__actions">
              {!runtime.available ? (
                <a
                  className="ar-button ar-button--default ar-button--primary"
                  href={config.windowsDownloadUrl}
                  rel="noreferrer"
                  target="_blank"
                >
                  <Download aria-hidden="true" />
                  {t('onboarding.runtime.download')}
                </a>
              ) : expectedTarget !== null && !runtimeMatches ? (
                <Button
                  disabled={runtime.busy !== null}
                  icon={<PlugZap aria-hidden="true" />}
                  onClick={() => void runtime.configureAgentRuntime(expectedTarget)}
                  tone="primary"
                >
                  {t('onboarding.runtime.configure')}
                </Button>
              ) : runtime.snapshot?.bridge.authorization !== null ? (
                <Button
                  disabled={runtime.busy !== null}
                  icon={<ExternalLink aria-hidden="true" />}
                  onClick={() => {
                    const prompt = runtime.snapshot?.bridge.authorization;
                    if (prompt !== null && prompt !== undefined) {
                      void runtime.openAuthorization(prompt.promptId);
                    }
                  }}
                  tone="network"
                >
                  {t('desktop.authorization.open')}
                </Button>
              ) : null}
            </div>
          </FactCard>
        </div>

        <section className="onboarding__hosts">
          <header>
            <div>
              <p className="eyebrow">{t('onboarding.hosts.eyebrow')}</p>
              <h2>{t('onboarding.hosts')}</h2>
            </div>
            <p>{t('onboarding.hosts.detail')}</p>
          </header>
          {runtime.available && installedHosts.length > 0 ? (
            <div className="onboarding__host-list">
              {installedHosts.map((host) => (
                <Button
                  disabled={runtime.busy !== null || !host.configurable}
                  icon={<PlugZap aria-hidden="true" />}
                  key={host.host}
                  onClick={() => void runtime.configureHost(host.host)}
                  tone="ghost"
                >
                  {t('onboarding.hosts.configure', { host: hostNames[host.host] })}
                </Button>
              ))}
            </div>
          ) : (
            <p className="onboarding__empty">{t('onboarding.hosts.none')}</p>
          )}
        </section>

        {bootstrap.data?.ok === false ||
        runtime.failure !== null ||
        entry.data?.ok === false ||
        entry.isError ? (
          <section aria-live="assertive" className="onboarding__failure">
            <KeyRound aria-hidden="true" />
            <div>
              <strong>{t('onboarding.failure')}</strong>
              <code>
                {bootstrap.data?.ok === false
                  ? bootstrap.data.error.code
                  : (runtime.failure?.code ??
                    (entry.data?.ok === false
                      ? entry.data.error.code
                      : entry.isError
                        ? 'lobby_entry.unexpected_failure'
                        : undefined))}
              </code>
            </div>
            <Button
              icon={<RefreshCw aria-hidden="true" />}
              onClick={() => {
                entry.reset();
                void bootstrap.refetch();
              }}
              tone="alert"
            >
              {t('onboarding.retry')}
            </Button>
          </section>
        ) : null}
      </section>

      <footer className="onboarding__footer">
        <a href="/connect">{t('onboarding.signOut')}</a>
        {resolved === null ? null : (
          <Button
            disabled={entry.isPending || (runtime.available && !runtimeReady)}
            icon={<ArrowRight aria-hidden="true" />}
            onClick={() => {
              if (runtime.available && bridgeSession !== null) {
                entry.mutate({
                  kind: 'known',
                  target: {
                    catalogId: resolved.lobby.catalogId,
                    matrixRoomId: bridgeSession.matrixRoomId,
                  },
                });
                return;
              }
              entry.mutate({ kind: 'resolve', catalogId: resolved.lobby.catalogId });
            }}
            size="large"
            tone="primary"
          >
            {t(runtime.available ? 'onboarding.continue' : 'onboarding.webContinue')}
          </Button>
        )}
      </footer>
    </main>
  );
}

type LobbyEntryIntent =
  | { readonly catalogId: string; readonly kind: 'resolve' }
  | { readonly kind: 'known'; readonly target: PublicLobbyRouteTarget };

type FactCardProps = {
  readonly children: ReactNode;
  readonly detail: string;
  readonly icon: ReactNode;
  readonly status: 'active' | 'complete' | 'optional' | 'pending';
  readonly title: string;
};

function FactCard({ children, detail, icon, status, title }: FactCardProps) {
  return (
    <motion.article
      animate={{ opacity: 1, y: 0 }}
      className="onboarding-fact"
      data-status={status}
      initial={{ opacity: 0, y: 10 }}
      transition={{ damping: 28, stiffness: 260, type: 'spring' }}
    >
      <div className="onboarding-fact__icon">{icon}</div>
      <div className="onboarding-fact__copy">
        <div>
          <h2>{title}</h2>
          <StatusMark label={status} tone={status === 'complete' ? 'active' : 'network'} />
        </div>
        <p>{detail}</p>
        <div className="onboarding-fact__value">{children}</div>
      </div>
    </motion.article>
  );
}

function runtimeDetail(
  available: boolean,
  matches: boolean,
  phase: OnboardingPhase,
  copy: {
    readonly authorization: string;
    readonly configured: string;
    readonly configuring: string;
    readonly waiting: string;
    readonly web: string;
  },
): string {
  if (!available) return copy.web;
  if (!matches) return copy.configuring;
  if (phase === 'authorizing-runtime') return copy.authorization;
  if (phase !== 'ready') return copy.waiting;
  return copy.configured;
}
