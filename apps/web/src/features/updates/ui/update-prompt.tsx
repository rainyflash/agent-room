import { Button } from '@agent-room/ui-system';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useState } from 'react';

import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';

export function UpdatePrompt() {
  const { t } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const [failure, setFailure] = useState(false);
  const [applying, setApplying] = useState(false);

  if (!runtime.updateWaiting) {
    return null;
  }

  return (
    <aside aria-live="polite" className="update-prompt">
      <div>
        <strong>{t('pwa.update.title')}</strong>
        <p>{t(runtime.writes.allowed ? 'pwa.update.compatible' : 'pwa.update.writeBlocked')}</p>
        {failure ? <p role="alert">{t('pwa.update.failed')}</p> : null}
      </div>
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
    </aside>
  );
}
