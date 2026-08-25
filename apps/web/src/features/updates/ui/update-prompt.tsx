import { Button } from '@agent-room/ui-system';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';

export function UpdatePrompt() {
  const { t } = useTranslation();
  const runtime = useRuntimeCompatibility();

  if (!runtime.updateWaiting) {
    return null;
  }

  return (
    <aside aria-live="polite" className="update-prompt">
      <div>
        <strong>{t('pwa.update.title')}</strong>
        <p>{t('pwa.update.writeBlocked')}</p>
      </div>
      <Button
        icon={<RefreshCw aria-hidden="true" />}
        onClick={() => void runtime.applyUpdate()}
        size="compact"
        tone="primary"
      >
        {t('pwa.update.action')}
      </Button>
    </aside>
  );
}
