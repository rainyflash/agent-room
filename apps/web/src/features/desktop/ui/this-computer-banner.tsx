import { Banner, Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { ExternalLink, RotateCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { authorizationFailureMessage, haltReasonMessage } from '../domain/desktop-connection';
import { useOptionalDesktopRuntimeController } from './desktop-runtime-provider';
import './this-computer.css';

/**
 * 这台电脑要授权或者连接停了时，所有页面上方都出一条提示，一个按钮就能处理。
 * 细节在“我的 Agent”的“这台电脑”一节，那一页自己会显示，这里就不再重复。
 */
export function ThisComputerBanner({ hidden = false }: { readonly hidden?: boolean }) {
  const { i18n, t } = useTranslation();
  const controller = useOptionalDesktopRuntimeController();
  if (hidden || controller?.available !== true) return null;
  const bridge = controller.snapshot?.bridge;
  const authorization = bridge?.authorization ?? null;
  if (authorization !== null) {
    const time = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(authorization.expiresAtUnixMs));
    return (
      <div className="this-computer-banner">
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
          {t('thisComputer.authorize.code', {
            code: authorization.userCode,
            host: authorization.verificationHost,
            time,
          })}
        </Banner>
      </div>
    );
  }
  const lifecycle = bridge?.lifecycle;
  if (lifecycle?.phase !== 'halted') return null;
  const reason =
    lifecycle.diagnosticCode === 'desktop.authorization.failed'
      ? authorizationFailureMessage(lifecycle.lastFailureCode)
      : haltReasonMessage(lifecycle.diagnosticCode);
  return (
    <div className="this-computer-banner">
      <Banner
        action={
          <>
            <Button
              disabled={controller.busy !== null}
              icon={<RotateCcw aria-hidden="true" />}
              onClick={() => void controller.retryBridge()}
              size="compact"
              tone="alert"
            >
              {t('desktop.halted.retry')}
            </Button>
            <Link
              className="ar-button ar-button--compact ar-button--ghost"
              hash="this-computer"
              search={{}}
              to="/workspace"
            >
              {t('thisComputer.open')}
            </Link>
          </>
        }
        title={t('thisComputer.stopped.title')}
        tone="danger"
      >
        {t(reason)}
      </Banner>
    </div>
  );
}
