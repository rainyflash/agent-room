import { Link } from '@tanstack/react-router';
import {
  Banner,
  Button,
  Details,
  Spinner,
  StatusMark,
  type StatusTone,
} from '@agent-room/ui-system';
import { ArrowRight, Clipboard, LogIn, LogOut, RefreshCw, ShieldCheck } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { RoomIllustration } from '@/features/lobby/ui/room-illustration';
import type { ConnectionAction, ConnectionStage, ConnectionViewModel } from './connection-model';
import type { SessionContext } from '@/features/session/domain/session-machine';
import type { TranslationKey } from '@/shared/i18n/resources';

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

const toneByStage: Readonly<Record<ConnectionStage['status'], StatusTone>> = {
  blocked: 'alert',
  complete: 'active',
  current: 'network',
  pending: 'idle',
};

const stageStatusKey: Readonly<Record<ConnectionStage['status'], TranslationKey>> = {
  blocked: 'connection.stage.status.blocked',
  complete: 'connection.stage.status.complete',
  current: 'connection.stage.status.current',
  pending: 'connection.stage.status.pending',
};

/**
 * 连接页只有一张卡片：欢迎、要做的事（通常是登录）、此刻在做什么的一行字。五段进度、账户
 * 和设备标识、各项服务的状态都收在“连接详情”里，排查时再看。
 */
export function ConnectionWorkspace({
  children,
  context,
  onAction,
  view,
}: ConnectionWorkspaceProps) {
  const { t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const failure = context.failure;
  const action = view.action;
  const currentStage = view.stages[view.currentStage];

  return (
    <main className="entry-card connection-workspace" id="main-content">
      <div aria-hidden="true" className="connection-workspace__art">
        <RoomIllustration populated />
      </div>
      <motion.section
        animate={{ y: 0 }}
        aria-labelledby="connection-state-title"
        className="connection-workspace__stage"
        initial={reduceMotion === true ? false : { y: 10 }}
        key={`${view.state}-${context.authenticationTarget}`}
        transition={{ bounce: 0.16, damping: 24, stiffness: 210, type: 'spring' }}
      >
        <h1 id="connection-state-title">{t(view.titleKey)}</h1>
        <p className="entry-card__lede">{t(view.detailKey)}</p>

        {view.busy && currentStage !== undefined ? (
          <p className="connection-now">
            <Spinner />
            <span>
              {t(currentStage.titleKey)} · {t(currentStage.detailKey)}
            </span>
          </p>
        ) : null}

        {failure === null ? null : (
          <Banner className="connection-failure" role="alert" tone="danger">
            {view.failureKey === null ? t('connection.state.failure.generic') : t(view.failureKey)}
          </Banner>
        )}

        <div className="entry-card__actions">
          {action === null || view.actionKey === null ? (
            <p aria-live="polite" className="operation-indicator">
              <Spinner />
              {t('connection.operation.active')}
            </p>
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
          {context.principal !== null && !view.busy && action !== 'logout' ? (
            <Button
              icon={actionIcons.logout}
              onClick={() => {
                onAction('logout');
              }}
              size="large"
              tone="quiet"
            >
              {t('connection.action.logout')}
            </Button>
          ) : null}
        </div>

        {context.principal === null ? null : (
          <p className="connection-identity">
            <ShieldCheck aria-hidden="true" />
            <span>{t('connection.signedInAs', { name: context.principal.displayName })}</span>
            <Link className="connection-account-link" to="/workspace" search={{}}>
              {t('connection.accountLink')}
              <ArrowRight aria-hidden="true" />
            </Link>
          </p>
        )}

        <p className="session-note">
          {view.state === 'offline'
            ? t('connection.note.offline')
            : view.state === 'degraded'
              ? t('connection.note.degraded')
              : t('connection.note.sso')}
        </p>
      </motion.section>
      <Details
        className="entry-card__details connection-service-details"
        summary={t('connection.detailsTitle')}
      >
        <ol aria-label={t('connection.progress')} className="connection-steps">
          {view.stages.map((stage) => (
            <li className={`connection-step connection-step--${stage.status}`} key={stage.titleKey}>
              <StatusMark
                label={t(stageStatusKey[stage.status])}
                pulse={stage.status === 'current'}
                tone={toneByStage[stage.status]}
              />
              <div>
                <strong>{t(stage.titleKey)}</strong>
                <span>{t(stage.detailKey)}</span>
              </div>
            </li>
          ))}
        </ol>
        <ConnectionDiagnostics context={context} />
        {children}
      </Details>
    </main>
  );
}

/** 账户 ID、Matrix ID、设备和出错的地方：排查才要看的标识。 */
function ConnectionDiagnostics({ context }: { readonly context: SessionContext }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState<'account' | 'diagnostic' | null>(null);
  const failure = context.failure;
  const copy = (value: string, target: 'account' | 'diagnostic'): void => {
    void navigator.clipboard.writeText(value).then(
      () => {
        setCopied(target);
      },
      () => {
        setCopied(null);
      },
    );
  };
  if (context.principal === null && failure === null) return null;
  return (
    <dl className="connection-diagnostics">
      {context.principal === null ? null : (
        <>
          <div>
            <dt>{t('connection.accountId')}</dt>
            <dd className="connection-diagnostics__account-id">
              <code>{context.principal.principalId}</code>
              <Button
                aria-label={t(
                  copied === 'account'
                    ? 'connection.accountId.copied'
                    : 'connection.accountId.copy',
                )}
                icon={<Clipboard aria-hidden="true" />}
                onClick={() => {
                  copy(context.principal?.principalId ?? '', 'account');
                }}
                size="compact"
                tone="quiet"
              >
                {t(
                  copied === 'account'
                    ? 'connection.accountId.copied'
                    : 'connection.accountId.copy',
                )}
              </Button>
            </dd>
            <dd className="connection-diagnostics__hint">{t('connection.accountId.hint')}</dd>
          </div>
          <div>
            <dt>{t('connection.matrixIdentity')}</dt>
            <dd>{context.principal.matrixUserId}</dd>
          </div>
          <div>
            <dt>{t('connection.device')}</dt>
            <dd>{context.connection?.deviceId ?? t('connection.device.pending')}</dd>
          </div>
        </>
      )}
      {failure === null ? null : (
        <>
          <div>
            <dt>{t('connection.failureBoundary')}</dt>
            <dd>{failure.boundary}</dd>
          </div>
          <div>
            <dt>{t('connection.errorCode')}</dt>
            <dd>{failure.code}</dd>
          </div>
          {failure.correlationId === undefined ? null : (
            <div>
              <dt>{t('connection.diagnosticId')}</dt>
              <dd className="connection-diagnostics__account-id">
                <code>{failure.correlationId}</code>
                <Button
                  icon={<Clipboard aria-hidden="true" />}
                  onClick={() => {
                    copy(failure.correlationId ?? '', 'diagnostic');
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {copied === 'diagnostic'
                    ? t('connection.action.copied')
                    : t('connection.action.details')}
                </Button>
              </dd>
            </div>
          )}
        </>
      )}
    </dl>
  );
}
