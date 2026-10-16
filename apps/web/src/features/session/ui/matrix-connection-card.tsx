import { Button, Spinner } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { RotateCw, TriangleAlert } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { sessionStateName } from '@/features/session/ui/connection-model';
import { matrixConnectionNotice } from '@/features/session/ui/matrix-connection-notice';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { EntryCard, EntryShell } from '@/shared/ui/entry-shell';

export type MatrixConnectionCardProps = {
  /** 没有会话（夹具）或者会话说已经连上时，照旧显示这个。 */
  readonly fallback: ReactNode;
};

/**
 * 房间页读不到 Matrix 客户端时的卡片。照会话此刻的状态说：正在连，
 * 或者要人点一下（重新连接、重新登录）。按钮交给会话去做，而不是再读一遍同一个空客户端；
 * 连上以后房间自己打开。
 */
export function MatrixConnectionCard({ fallback }: MatrixConnectionCardProps) {
  const { t } = useTranslation();
  const session = useOptionalSession();
  if (session === null) return <>{fallback}</>;
  const { send, snapshot } = session;
  const notice = matrixConnectionNotice(sessionStateName(snapshot.value), snapshot.context);
  if (notice === null) return <>{fallback}</>;
  const backToRooms = (
    <Link className="ar-button ar-button--large ar-button--ghost" to="/rooms">
      {t('entry.backToRooms')}
    </Link>
  );
  if (notice.kind === 'connecting') {
    return (
      <EntryShell>
        <EntryCard
          detail={t('matrixConnection.connecting.detail')}
          icon={<Spinner />}
          title={t('matrixConnection.connecting.title')}
        />
      </EntryShell>
    );
  }
  const failureCode = snapshot.context.failure?.code;
  return (
    <EntryShell>
      <EntryCard
        actions={
          <>
            {notice.action === null || notice.actionKey === null ? (
              <Link className="ar-button ar-button--large ar-button--primary" to="/connect">
                {t('matrixConnection.details')}
              </Link>
            ) : (
              <Button
                icon={<RotateCw aria-hidden="true" />}
                onClick={() => {
                  send({ type: notice.action === 'login' ? 'LOGIN' : 'RETRY' });
                }}
                size="large"
                tone="primary"
              >
                {t(notice.actionKey)}
              </Button>
            )}
            {backToRooms}
          </>
        }
        detail={t(notice.failureKey ?? 'matrixConnection.required.detail')}
        details={
          <dl>
            <div>
              <dt>{t('entry.errorCode')}</dt>
              <dd>
                <code>{failureCode ?? 'lobby.matrix_unavailable'}</code>
              </dd>
            </div>
          </dl>
        }
        icon={<TriangleAlert />}
        title={t('matrixConnection.required.title')}
        tone="alert"
      />
    </EntryShell>
  );
}
