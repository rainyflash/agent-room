import { Banner, Button, Details } from '@agent-room/ui-system';
import { ExternalLink, KeyRound, Monitor, RefreshCw, RotateCcw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useOptionalSession } from '@/features/session/ui/session-provider';
import {
  authorizationFailureMessage,
  haltReasonMessage,
  thisComputerState,
} from '../domain/desktop-connection';
import type { BridgeRuntime } from '../domain/desktop-runtime';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { LocalAgentSessions } from './local-agent-sessions';
import { LocalConnectionNotice } from './local-connection-notice';
import { ReceptionPanel } from './reception-panel';
import { ThisComputerSettings } from './this-computer-settings';
import './this-computer.css';

/**
 * “我的 Agent”里的“这台电脑”一节，只在桌面端出现：连接状态和要做的事、这台电脑上的 Agent、
 * 后台回复，以及这台电脑的设置。原来右下角浮着的“本机 Agent”面板里的东西都在这里。
 */
export function ThisComputerSection() {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const session = useOptionalSession();
  if (!controller.available) return null;
  const state = thisComputerState(controller.snapshot?.bridge);
  return (
    <section aria-labelledby="this-computer-title" className="this-computer" id="this-computer">
      <header className="this-computer__header">
        <span aria-hidden="true" className="this-computer__icon">
          <Monitor />
        </span>
        <div>
          <h2 id="this-computer-title">{t('thisComputer.title')}</h2>
          <p>{t('thisComputer.description')}</p>
        </div>
        <span className="this-computer__state" data-state={state}>
          {t(`thisComputer.state.${state}`)}
        </span>
      </header>
      <ThisComputerConnection />
      <div className="this-computer__columns">
        <LocalAgentSessions readHostSessions={controller.readHostSessions} />
        {controller.receptionAvailable ? (
          session === null ? (
            <p className="this-computer__muted">{t('reception.login')}</p>
          ) : (
            <ReceptionPanel />
          )
        ) : null}
      </div>
      <ThisComputerSettings />
    </section>
  );
}

/** 连接状态：要授权给代码和按钮，停了给原因和重试，正在连就说一声；排查用的代码收进详情。 */
function ThisComputerConnection() {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const bridge = controller.snapshot?.bridge;
  const phase = bridge?.lifecycle.phase ?? 'discovering';
  const failure = controller.failure;
  return (
    <>
      {bridge?.authorization != null ? (
        <AuthorizationPrompt authorization={bridge.authorization} />
      ) : phase === 'halted' && bridge !== undefined ? (
        <HaltedConnection bridge={bridge} />
      ) : phase === 'ready' || phase === 'authorized' ? (
        <Banner role={null} tone="success">
          {t('thisComputer.connected')}
        </Banner>
      ) : (
        <LocalConnectionNotice />
      )}
      {failure === null ? null : (
        <Banner
          action={
            failure.retryable ? (
              <Button
                disabled={controller.busy !== null}
                icon={<RefreshCw aria-hidden="true" />}
                onClick={() => void controller.refresh()}
                size="compact"
                tone="ghost"
              >
                {t('desktop.failure.refresh')}
              </Button>
            ) : (
              <Button
                icon={<X aria-hidden="true" />}
                onClick={controller.dismissFailure}
                size="compact"
                tone="ghost"
              >
                {t('desktop.failure.dismiss')}
              </Button>
            )
          }
          tone="danger"
        >
          <p>
            {t(
              failure.code === 'desktop.update.draft_unsaved'
                ? 'conversation.draftUnavailable'
                : 'thisComputer.failure',
            )}
          </p>
          <Details summary={t('connection.details')}>
            <code>{failure.code}</code>
          </Details>
        </Banner>
      )}
    </>
  );
}

function AuthorizationPrompt({
  authorization,
}: {
  readonly authorization: NonNullable<BridgeRuntime['authorization']>;
}) {
  const { i18n, t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const time = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(authorization.expiresAtUnixMs));
  return (
    <Banner
      action={
        <Button
          disabled={controller.busy !== null}
          icon={<ExternalLink aria-hidden="true" />}
          onClick={() => void controller.openAuthorization(authorization.promptId)}
          size="compact"
          tone="primary"
        >
          {t('desktop.authorization.open')}
        </Button>
      }
      title={t('thisComputer.authorize.title')}
      tone="warning"
    >
      <p>{t('thisComputer.authorize.body')}</p>
      <dl className="this-computer__code">
        <div>
          <dt>{t('desktop.authorization.code')}</dt>
          <dd className="this-computer__user-code">{authorization.userCode}</dd>
        </div>
        <div>
          <dt>{t('desktop.authorization.host')}</dt>
          <dd>{authorization.verificationHost}</dd>
        </div>
      </dl>
      <p className="this-computer__muted">{t('desktop.authorization.expires', { time })}</p>
    </Banner>
  );
}

/** 自动重启停下或授权没完成：说明原因，给重试；能重新授权时多一个按钮。 */
function HaltedConnection({ bridge }: { readonly bridge: BridgeRuntime }) {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const lifecycle = bridge.lifecycle;
  const authorizationFailed = lifecycle.diagnosticCode === 'desktop.authorization.failed';
  const codes = [
    lifecycle.diagnosticCode,
    lifecycle.lastFailureCode === lifecycle.diagnosticCode ? null : lifecycle.lastFailureCode,
  ].filter((code) => code !== null);
  return (
    <Banner
      action={
        <Button
          disabled={controller.busy !== null}
          icon={<RotateCcw aria-hidden="true" />}
          onClick={() => void controller.retryBridge()}
          size="compact"
          tone="alert"
        >
          {t('desktop.halted.retry')}
        </Button>
      }
      title={t(authorizationFailed ? 'desktop.authorization.failedTitle' : 'desktop.halted.title')}
      tone="danger"
    >
      <p>
        {t(
          authorizationFailed
            ? authorizationFailureMessage(lifecycle.lastFailureCode)
            : haltReasonMessage(lifecycle.diagnosticCode),
        )}
      </p>
      {bridge.deviceReauthorizationAvailable ? (
        <div className="this-computer__reauthorize">
          <p>{t('desktop.halted.reauthorizeDescription')}</p>
          <Button
            disabled={controller.busy !== null}
            icon={<KeyRound aria-hidden="true" />}
            onClick={() => void controller.reauthorizeBridge()}
            size="compact"
            tone="quiet"
          >
            {t('desktop.halted.reauthorize')}
          </Button>
        </div>
      ) : null}
      {codes.length === 0 && lifecycle.lastExitCode === null ? null : (
        <Details summary={t('connection.details')}>
          {codes.map((code) => (
            <code key={code}>{code}</code>
          ))}
          {lifecycle.lastExitCode === null ? null : (
            <p>
              {t('desktop.halted.exitCode')} <code>{lifecycle.lastExitCode}</code>
            </p>
          )}
        </Details>
      )}
    </Banner>
  );
}
