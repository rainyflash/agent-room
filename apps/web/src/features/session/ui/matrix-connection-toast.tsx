import { Button, Toast } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { sessionStateName } from '@/features/session/ui/connection-model';
import { matrixConnectionNotice } from '@/features/session/ui/matrix-connection-notice';
import { useSession } from '@/features/session/ui/session-provider';

export type MatrixConnectionToastProps = {
  /** 连接页、安全设置、房间页自己会说，就不再提示。 */
  readonly hidden: boolean;
};

/**
 * 账户登录着、这台设备的消息却没连上，而且要人动手时，提示栈里常驻一条：
 * 连接出错或要重新登录时给“重新连接”或“登录”。正在连的时候不打扰。
 */
export function MatrixConnectionToast({ hidden }: MatrixConnectionToastProps) {
  const { t } = useTranslation();
  const { send, snapshot } = useSession();
  const notice = matrixConnectionNotice(sessionStateName(snapshot.value), snapshot.context);
  if (hidden || notice === null || notice.kind === 'connecting') return null;
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
