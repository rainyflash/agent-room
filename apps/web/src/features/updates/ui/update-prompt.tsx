import { Button, Toast } from '@agent-room/ui-system';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useState } from 'react';

import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';

/** 网页端有新版本在等：提示栈里一条，按一下重新载入。 */
export function UpdatePrompt() {
  const { t } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const [failure, setFailure] = useState(false);
  const [applying, setApplying] = useState(false);

  if (!runtime.updateWaiting) {
    return null;
  }

  return (
    <Toast
      action={
        <Button
          icon={<RefreshCw aria-hidden="true" />}
          disabled={applying}
          onClick={() => {
            setApplying(true);
            setFailure(false);
            void runtime
              .applyUpdate()
              .catch(() => {
                setFailure(true);
              })
              .finally(() => {
                setApplying(false);
              });
          }}
          size="compact"
          tone="primary"
        >
          {t('pwa.update.action')}
        </Button>
      }
      title={t('pwa.update.title')}
      tone={failure ? 'danger' : 'info'}
    >
      <p>{t(runtime.writes.allowed ? 'pwa.update.compatible' : 'pwa.update.writeBlocked')}</p>
      {failure ? <p role="alert">{t('pwa.update.failed')}</p> : null}
    </Toast>
  );
}
