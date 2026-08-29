import { Button, StatusMark } from '@agent-room/ui-system';
import { useNavigate } from '@tanstack/react-router';
import { ExternalLink, KeyRound, RefreshCw, ShieldCheck, TerminalSquare } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { desktopConnectionView } from '@/features/desktop/domain/desktop-connection';
import { useDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { ConnectionRail } from '@/features/session/ui/connection-rail';

import './desktop-connection-page.css';

export function DesktopConnectionPage() {
  const { i18n, t } = useTranslation();
  const navigate = useNavigate();
  const reduceMotion = useReducedMotion();
  const runtime = useDesktopRuntimeController();
  const bootstrapDefaultAgent = runtime.bootstrapDefaultAgent;
  const phase = runtime.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const autoBootstrapAttempt = useRef(false);
  const lobbyNavigationAttempt = useRef(false);
  const failed = runtime.failure !== null || phase === 'halted' || phase === 'stopped';
  const view = useMemo(
    () => desktopConnectionView(phase, runtime.busy !== null, failed),
    [failed, phase, runtime.busy],
  );
  const authorization = runtime.snapshot?.bridge.authorization ?? null;
  const target = runtime.snapshot?.agentTarget ?? null;
  const session = runtime.snapshot?.bridge.session ?? null;
  const failureCode =
    runtime.failure?.code ??
    runtime.snapshot?.bridge.lifecycle.lastFailureCode ??
    runtime.snapshot?.bridge.lifecycle.diagnosticCode ??
    null;

  useEffect(() => {
    if (phase !== 'authorized') {
      autoBootstrapAttempt.current = false;
      return;
    }
    if (
      target !== null ||
      runtime.busy !== null ||
      runtime.failure !== null ||
      autoBootstrapAttempt.current
    ) {
      return;
    }
    autoBootstrapAttempt.current = true;
    void bootstrapDefaultAgent(i18n.resolvedLanguage ?? null);
  }, [bootstrapDefaultAgent, i18n.resolvedLanguage, phase, runtime.busy, runtime.failure, target]);

  useEffect(() => {
    if (
      phase !== 'ready' ||
      target === null ||
      session === null ||
      target.agentId !== session.agentId
    ) {
      lobbyNavigationAttempt.current = false;
      return;
    }
    if (lobbyNavigationAttempt.current) return;
    lobbyNavigationAttempt.current = true;
    void navigate({
      params: {
        catalogId: target.publicLobbyCatalogId,
        roomId: session.matrixRoomId,
      },
      replace: true,
      search: {},
      to: '/lobby/$catalogId/instance/$roomId',
    });
  }, [navigate, phase, session, target]);

  return (
    <div className="connection-shell desktop-connection">
      <ConnectionRail
        sessionKey="desktop.connection.session"
        stages={view.stages}
        transportKey="desktop.connection.transport"
      />
      <main className="connection-workspace desktop-connection__workspace" id="main-content">
        <header className="connection-workspace__topbar">
          <p>{t('desktop.connection.eyebrow')}</p>
          <div className={`live-badge live-badge--${view.tone}`}>
            <StatusMark label={t(view.statusKey)} pulse={view.busy} tone={view.tone} />
            <span>{t(view.statusKey)}</span>
          </div>
        </header>

        <motion.section
          animate={{ opacity: 1, y: 0 }}
          aria-labelledby="desktop-connection-title"
          className="connection-workspace__stage desktop-connection__stage"
          initial={reduceMotion === true ? false : { opacity: 0, y: 14 }}
          key={phase}
          transition={{ bounce: 0.14, damping: 25, stiffness: 220, type: 'spring' }}
        >
          <div aria-hidden="true" className="stage-coordinate">
            {String(view.currentStage + 1).padStart(2, '0')}
          </div>
          <p className="eyebrow">{t('desktop.connection.current')}</p>
          <h1 id="desktop-connection-title">{t(view.titleKey)}</h1>
          <p className="connection-workspace__lede">{t(view.detailKey)}</p>

          {authorization === null ? null : (
            <section className="desktop-connection__authorization">
              <KeyRound aria-hidden="true" />
              <div>
                <span>{t('desktop.authorization.host')}</span>
                <strong>{authorization.verificationHost}</strong>
              </div>
              <div>
                <span>{t('desktop.authorization.code')}</span>
                <code>{authorization.userCode}</code>
              </div>
              <Button
                disabled={runtime.busy !== null}
                icon={<ExternalLink aria-hidden="true" />}
                onClick={() => void runtime.openAuthorization(authorization.promptId)}
                size="large"
                tone="network"
              >
                {t('desktop.authorization.open')}
              </Button>
            </section>
          )}

          {failureCode === null ? null : (
            <aside className="failure-panel" role="alert">
              <span className="failure-panel__line" />
              <div>
                <p>{t('desktop.connection.failure')}</p>
                <code>{failureCode}</code>
              </div>
            </aside>
          )}

          <div className="connection-actions">
            {phase === 'halted' || phase === 'stopped' ? (
              <Button
                disabled={runtime.busy !== null}
                icon={<RefreshCw aria-hidden="true" />}
                onClick={() => void runtime.retryBridge()}
                size="large"
                tone="alert"
              >
                {t('desktop.connection.retry')}
              </Button>
            ) : phase === 'authorized' && runtime.failure !== null ? (
              <Button
                disabled={runtime.busy !== null}
                icon={<RefreshCw aria-hidden="true" />}
                onClick={() => {
                  autoBootstrapAttempt.current = true;
                  void bootstrapDefaultAgent(i18n.resolvedLanguage ?? null);
                }}
                size="large"
                tone="alert"
              >
                {t('desktop.connection.retry')}
              </Button>
            ) : view.busy ? (
              <div aria-live="polite" className="operation-indicator">
                <span />
                {t('desktop.connection.operation')}
              </div>
            ) : null}
          </div>

          <section
            aria-label={t('desktop.connection.identity')}
            className="identity-summary desktop-connection__identity"
          >
            <div className="identity-summary__icon">
              {session === null ? (
                <TerminalSquare aria-hidden="true" />
              ) : (
                <ShieldCheck aria-hidden="true" />
              )}
            </div>
            <div className="identity-summary__primary">
              <span>{t('desktop.connection.identity')}</span>
              <strong>
                {session === null
                  ? t('desktop.connection.identityPending')
                  : t('desktop.connection.identityReady')}
              </strong>
            </div>
            <dl>
              <div>
                <dt>{t('desktop.connection.agent')}</dt>
                <dd>{target?.agentId ?? '—'}</dd>
              </div>
              <div>
                <dt>{t('desktop.connection.room')}</dt>
                <dd>{session?.matrixRoomId ?? '—'}</dd>
              </div>
            </dl>
          </section>

          <p className="session-note">{t('desktop.connection.note')}</p>
        </motion.section>
      </main>
    </div>
  );
}
