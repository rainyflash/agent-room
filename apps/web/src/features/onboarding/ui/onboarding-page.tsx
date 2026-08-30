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
  projectOnboardingRuntimePhase,
  targetFor,
  targetMatches,
  type OnboardingRuntimePhase,
} from '@/features/onboarding/domain/onboarding';
import { useDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import type { WebSession } from '@/features/session/domain/session';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { ConnectionPage } from '@/features/session/ui/connection-page';
import { useSession } from '@/features/session/ui/session-provider';

import './onboarding-page.css';

const hostNames = {
  codex: 'Codex',
  'claude-code': 'Claude Code',
  cursor: 'Cursor',
} as const;

export function OnboardingPage() {
  const { snapshot } = useSession();
  const principal = snapshot.context.principal;
  if (sessionStateName(snapshot.value) !== 'ready' || principal === null) {
    return <ConnectionPage />;
  }
  return <ReadyOnboardingPage principal={principal} />;
}

function ReadyOnboardingPage({ principal }: { readonly principal: WebSession }) {
  const { i18n, t } = useTranslation();
  const navigate = useNavigate();
  const reduceMotion = useReducedMotion();
  const { config, lobbyEntry, onboarding } = useAppServices();
  const locale = principal.locale ?? i18n.resolvedLanguage ?? 'en';
  const bootstrap = useQuery({
    networkMode: 'always',
    queryFn: async () => await onboarding.bootstrap(locale),
    queryKey: ['onboarding', 'bootstrap', principal.principalId, locale] as const,
    retry: false,
    staleTime: 5_000,
  });
  const runtime = useDesktopRuntimeController();
  const resolved = bootstrap.data?.ok === true ? bootstrap.data.value : null;
  const expectedTarget =
    resolved === null ? null : targetFor(resolved.agent, resolved.lobby, locale);
  const runtimeMatches =
    expectedTarget !== null && targetMatches(runtime.snapshot?.agentTarget ?? null, expectedTarget);
  const bridgeSession = runtime.snapshot?.bridge.session ?? null;
  const runtimeSessionReady = bridgeSession?.agentId === resolved?.agent.agentId;
  const runtimePhase = projectOnboardingRuntimePhase({
    bridgePhase: runtime.snapshot?.bridge.lifecycle.phase ?? null,
    desktopAvailable: runtime.available,
    runtimeSessionReady,
    targetMatches: runtimeMatches,
  });
  const runtimeReady = runtimePhase === 'ready';
  const entry = useMutation({
    mutationFn: async (target: PublicLobbyRouteTarget) => await lobbyEntry.enterKnown(target),
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
    accountReady: true,
    bootstrapFailed: bootstrap.data?.ok === false || bootstrap.isError,
    bootstrapReady: resolved !== null,
  });
  const installedHosts = runtime.hosts.filter((host) => host.installed);
  const progress = {
    account: true,
    agent: resolved !== null,
    runtime: runtimeReady,
  } as const;

  return (
    <main className="onboarding" id="main-content">
      <header className="onboarding__topbar">
        <a aria-label={t('app.name')} className="onboarding__brand" href="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <div className="onboarding__operator">
          <span>{t(`onboarding.phase.${phase}`)}</span>
          <strong>{principal.displayName}</strong>
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
              <li data-complete={progress[step] ? 'true' : 'false'} key={step}>
                <span>{progress[step] ? <Check aria-hidden="true" /> : `0${index + 1}`}</span>
                {t(`onboarding.${step}`)}
              </li>
            ))}
          </ol>
        </motion.header>

        <div className="onboarding__facts">
          <FactCard
            detail={t('onboarding.account.detail')}
            icon={<UserRound aria-hidden="true" />}
            status="complete"
            title={t('onboarding.account')}
          >
            <strong>{principal.displayName}</strong>
            <code>{principal.matrixUserId}</code>
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
            detail={runtimeDetail(runtimePhase, {
              authorization: t('onboarding.runtime.authorization'),
              configured: t('onboarding.runtime.configured', {
                agent: resolved?.agent.displayName ?? 'Agent',
              }),
              configuring: t('onboarding.phase.configuring-runtime'),
              failed: t('onboarding.runtime.failed'),
              waiting: t('onboarding.runtime.waiting'),
              web: t('onboarding.runtime.web'),
            })}
            icon={<PlugZap aria-hidden="true" />}
            status={runtimeReady ? 'complete' : runtime.available ? 'active' : 'optional'}
            title={t('onboarding.runtime')}
          >
            <div className="onboarding__actions">
              {!runtime.available ? (
                config.windowsDownloadUrl === null ? (
                  <button
                    className="ar-button ar-button--default ar-button--primary"
                    disabled
                    type="button"
                  >
                    <Download aria-hidden="true" />
                    {t('onboarding.runtime.downloadPending')}
                  </button>
                ) : (
                  <a
                    className="ar-button ar-button--default ar-button--primary"
                    href={config.windowsDownloadUrl}
                    rel="noreferrer"
                    target="_blank"
                  >
                    <Download aria-hidden="true" />
                    {t('onboarding.runtime.download')}
                  </a>
                )
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
            disabled={entry.isPending}
            icon={<ArrowRight aria-hidden="true" />}
            onClick={() => {
              if (runtimeReady && bridgeSession !== null) {
                entry.mutate({
                  catalogId: resolved.lobby.catalogId,
                  matrixRoomId: bridgeSession.matrixRoomId,
                });
                return;
              }
              void navigate({
                params: { catalogId: resolved.lobby.catalogId },
                search: {},
                to: '/lobby/$catalogId',
              });
            }}
            size="large"
            tone="primary"
          >
            {t('onboarding.continue')}
          </Button>
        )}
      </footer>
    </main>
  );
}

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
  phase: OnboardingRuntimePhase,
  copy: {
    readonly authorization: string;
    readonly configured: string;
    readonly configuring: string;
    readonly failed: string;
    readonly waiting: string;
    readonly web: string;
  },
): string {
  return runtimeCopyByPhase[phase](copy);
}

const runtimeCopyByPhase: Readonly<
  Record<OnboardingRuntimePhase, (copy: Parameters<typeof runtimeDetail>[1]) => string>
> = {
  'authorization-required': (copy) => copy.authorization,
  'configuration-required': (copy) => copy.configuring,
  connecting: (copy) => copy.waiting,
  failed: (copy) => copy.failed,
  optional: (copy) => copy.web,
  ready: (copy) => copy.configured,
};
