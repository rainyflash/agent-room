import { LoaderCircle, MessageCircle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { DirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import { AgentAvatar, agentTone } from '@/features/lobby/ui/avatars';

export function DirectSessionNavigation({
  activeCatalogId,
  controller,
  onActivate,
}: {
  readonly activeCatalogId: string | null;
  readonly controller: DirectSessionController;
  readonly onActivate: (catalogId: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <nav className="workspace-direct" aria-label={t('directSessions.rail.label')}>
      <p className="workspace-direct__label">
        {t('roomWorkspace.direct')}
        <MessageCircle aria-hidden="true" />
      </p>
      {controller.sessions.map((session) => (
        <button
          type="button"
          className="workspace-direct__session"
          key={session.catalogId}
          aria-pressed={activeCatalogId === session.catalogId}
          aria-label={t('directSessions.rail.open', { name: session.target.displayName })}
          onClick={() => {
            onActivate(session.catalogId);
          }}
        >
          <span className="workspace-direct__avatar">
            <AgentAvatar id={session.target.agentId} />
          </span>
          <span className="workspace-direct__identity">
            {/* 在左栏里（深色）样式不给它颜色；搬进浅底的下拉、抽屉里才跟着机器人。 */}
            <strong className="ar-name" data-tone={agentTone(session.target.agentId)}>
              {session.target.displayName}
            </strong>
            <small>{t(`directSessions.lifecycle.${session.lifecycle}`)}</small>
          </span>
        </button>
      ))}
      {controller.loading ? (
        <p className="workspace-direct__empty" role="status">
          <LoaderCircle aria-hidden="true" />
          {t('directSessions.loading.title')}
        </p>
      ) : controller.failure !== null ? (
        <div className="workspace-direct__empty" role="alert">
          <p>{t('roomWorkspace.directFailed')}</p>
          <button
            type="button"
            onClick={() => {
              void controller.retry();
            }}
          >
            {t('roomWorkspace.retry')}
          </button>
        </div>
      ) : controller.sessions.length === 0 ? (
        <p className="workspace-direct__empty">{t('roomWorkspace.noDirect')}</p>
      ) : null}
    </nav>
  );
}
