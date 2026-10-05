import { Button, Toast } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { sessionStateName } from '@/features/session/ui/connection-model';
import { matrixConnectionNotice } from '@/features/session/ui/matrix-connection-notice';
import { useSession } from '@/features/session/ui/session-provider';

export type MatrixConnectionToastProps = {
  /** 连接页、安全设置、房间页自己会说，就不再提示。 */
  readonly hidden: boolean;
};

/**
 * 账户登录着、这台设备的消息却没连上，而且要人动手时，提示栈里常驻一条：
 * 桌面端等着浏览器里登录完时说一声（登录页可能被别的窗口挡住了），
 * 连接出错或要重新登录时给“重新连接”或“登录”。正在连的时候不打扰。
 */
export function MatrixConnectionToast({ hidden }: MatrixConnectionToastProps) {
  const { t } = useTranslation();
  const { send, snapshot } = useSession();
  const desktop = useOptionalDesktopRuntimeController()?.available === true;
  const notice = matrixConnectionNotice(
    sessionStateName(snapshot.value),
    snapshot.context,
    desktop,
  );
  if (hidden || notice === null || notice.kind === 'connecting') return null;
  if (notice.kind === 'browserSignIn') {
    return (
      <Toast
        action={
          <Button
            onClick={() => {
              send({ type: 'RETRY' });
            }}
            size="compact"
          >
            {t('connection.action.restartLogin')}
          </Button>
        }
        role="status"
        title={t('matrixConnection.browserSignIn.title')}
      >
        {t('matrixConnection.browserSignIn.detail')}
      </Toast>
    );
  }
  return (
    <Toast
      action={
        notice.action === null || notice.actionKey === null ? (
          <Link className="ar-button ar-button--compact ar-button--primary" to="/connect">
            {t('matrixConnection.details')}
          </Link>
        ) : (
          <Button
            onClick={() => {
              send({ type: notice.action === 'login' ? 'LOGIN' : 'RETRY' });
            }}
            size="compact"
            tone="primary"
          >
            {t(notice.actionKey)}
          </Button>
        )
      }
      title={t('matrixConnection.required.title')}
      tone="warning"
    >
      {t(notice.failureKey ?? 'matrixConnection.required.detail')}
    </Toast>
  );
}
