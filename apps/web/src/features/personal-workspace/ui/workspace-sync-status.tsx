import { useTranslation } from 'react-i18next';
import { usePersonalWorkspace } from './personal-workspace-provider';

export function WorkspaceSyncStatus() {
  const workspace = usePersonalWorkspace();
  const { t } = useTranslation();
  if (!workspace || workspace.snapshot.status === 'unavailable') return null;
  const { status, failure } = workspace.snapshot;
  return (
    <div className="personal-sync" role={status === 'failed' ? 'alert' : 'status'}>
      <span>{t(`personal.sync.${status}`)}</span>
      {status === 'failed' ? (
        <>
          <button type="button" onClick={workspace.retry}>
            {t('personal.retry')}
          </button>
          <details>
            <summary>{t('connection.details')}</summary>
            <code>{failure?.code}</code>
          </details>
        </>
      ) : null}
    </div>
  );
}
