import { Button } from '@agent-room/ui-system';
import { RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useRegisterSW } from 'virtual:pwa-register/react';

export function UpdatePrompt() {
  const { t } = useTranslation();
  const {
    needRefresh: [needRefresh, setNeedRefresh],
    updateServiceWorker,
  } = useRegisterSW();

  if (!needRefresh) {
    return null;
  }

  return (
    <aside aria-live="polite" className="update-prompt">
      <p>{t('pwa.update.title')}</p>
      <Button
        icon={<RefreshCw aria-hidden="true" />}
        onClick={() => void updateServiceWorker(true)}
        size="compact"
        tone="primary"
      >
        {t('pwa.update.action')}
      </Button>
      <Button
        aria-label={t('pwa.update.dismiss')}
        icon={<X aria-hidden="true" />}
        onClick={() => {
          setNeedRefresh(false);
        }}
        size="compact"
        tone="quiet"
      >
        {t('pwa.update.dismiss')}
      </Button>
    </aside>
  );
}
