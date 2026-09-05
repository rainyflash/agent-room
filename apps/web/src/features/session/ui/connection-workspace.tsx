import { Button, StatusMark } from '@agent-room/ui-system';
import {
  ArrowRight,
  Clipboard,
  LogIn,
  LogOut,
  RefreshCw,
  ShieldCheck,
  TerminalSquare,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ConnectionAction, ConnectionViewModel } from './connection-model';
import type { SessionContext } from '@/features/session/domain/session-machine';

export type ConnectionWorkspaceProps = {
  readonly context: SessionContext;
  readonly onAction: (action: ConnectionAction) => void;
  readonly view: ConnectionViewModel;
  readonly children: React.ReactNode;
};

const actionIcons: Readonly<Record<ConnectionAction, React.ReactNode>> = {
  enter: <ArrowRight aria-hidden="true" />,
  login: <LogIn aria-hidden="true" />,
  logout: <LogOut aria-hidden="true" />,
  retry: <RefreshCw aria-hidden="true" />,
};

export function ConnectionWorkspace({
  children,
  context,
  onAction,
  view,
}: ConnectionWorkspaceProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const [copied, setCopied] = useState(false);
  const failure = context.failure;
  const action = view.action;

  return (
    <main className="connection-workspace" id="main-content">
      <header className="connection-workspace__topbar">
        <p>{t('connection.eyebrow')}</p>
        <div className={`live-badge live-badge--${view.tone}`}>
          <StatusMark label={t(view.statusKey)} pulse={view.busy} tone={view.tone} />
          <span>{t(view.statusKey)}</span>
        </div>
      </header>

      <motion.section
        animate={{ opacity: 1, y: 0 }}
        aria-labelledby="connection-state-title"
        className="connection-workspace__stage"
        initial={reduceMotion === true ? false : { opacity: 0, y: 14 }}
        key={`${view.state}-${context.authenticationTarget}`}
        transition={{ bounce: 0.16, damping: 24, stiffness: 210, type: 'spring' }}
      >
        <div className="stage-coordinate" aria-hidden="true">
          {String(view.currentStage + 1).padStart(2, '0')}
        </div>
        <p className="eyebrow">{t('connection.current')}</p>
        <h1 id="connection-state-title">{t(view.titleKey)}</h1>
        <p className="connection-workspace__lede">{t(view.detailKey)}</p>

        {failure === null ? null : (
          <aside className="failure-panel" role="alert">
            <span className="failure-panel__line" />
            <div>
              <p>
                {view.failureKey === null
                  ? t('connection.state.failure.generic')
                  : t(view.failureKey)}
              </p>
              <dl>
                <div>
                  <dt>{t('connection.failureBoundary')}</dt>
                  <dd>{failure.boundary}</dd>
                </div>
                <div>
                  <dt>{t('connection.errorCode')}</dt>
                  <dd>{failure.code}</dd>
                </div>
              </dl>
            </div>
          </aside>
        )}

        <div className="connection-actions">
          {action === null || view.actionKey === null ? (
            <div aria-live="polite" className="operation-indicator">
              <span />
              {t('connection.operation.active')}
            </div>
          ) : (
            <Button
              icon={actionIcons[action]}
              onClick={() => {
                onAction(action);
              }}
              size="large"
              tone={view.action === 'logout' ? 'alert' : 'primary'}
            >
              {t(view.actionKey)}
            </Button>
          )}
          {context.controlStatus === 'ready' && action !== 'enter' ? (
            <Button
              icon={<ArrowRight aria-hidden="true" />}
              onClick={() => {
                onAction('enter');
              }}
              size="large"
              tone="ghost"
            >
              {t('connection.action.openCloudWorkspace')}
            </Button>
          ) : null}
          {failure?.correlationId === undefined ? null : (
            <Button
              icon={<Clipboard aria-hidden="true" />}
              onClick={() => {
                void navigator.clipboard
                  .writeText(failure.correlationId ?? '')
                  .then(() => {
                    setCopied(true);
                  })
                  .catch(() => {
                    setCopied(false);
                  });
              }}
              size="large"
              tone="ghost"
            >
              {copied ? t('connection.action.copied') : t('connection.action.details')}
            </Button>
          )}
        </div>

        <section className="identity-summary" aria-label={t('connection.identity')}>
          <div className="identity-summary__icon">
            {context.principal === null ? (
              <TerminalSquare aria-hidden="true" />
            ) : (
              <ShieldCheck aria-hidden="true" />
            )}
          </div>
          <div className="identity-summary__primary">
            <span>{t('connection.identity')}</span>
            <strong>{context.principal?.displayName ?? t('connection.identity.pending')}</strong>
          </div>
          <dl>
            <div>
              <dt>{t('connection.matrixIdentity')}</dt>
              <dd>{context.principal?.matrixUserId ?? '—'}</dd>
            </div>
            <div>
              <dt>{t('connection.device')}</dt>
              <dd>{context.connection?.deviceId ?? t('connection.device.pending')}</dd>
            </div>
          </dl>
        </section>

        <p className="session-note">
          {view.state === 'offline'
            ? t('connection.note.offline')
            : view.state === 'degraded'
              ? t('connection.note.degraded')
              : t('connection.note.sso')}
        </p>
      </motion.section>
      {children}
    </main>
  );
}
