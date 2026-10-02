import { Banner, Button } from '@agent-room/ui-system';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { ThisDeviceState } from '@/features/security/domain/device-signing';

export type ThisDeviceStatusProps = {
  readonly state: ThisDeviceState;
  readonly onRetry: () => void;
};

/**
 * 这台设备：已就绪、正在准备，或者出错可重试。签名全自动（ADR 0011），这里不让人做任何选择，
 * 只在出错时给一个“重试”。
 */
export function ThisDeviceStatus({ state, onRetry }: ThisDeviceStatusProps) {
  const { t } = useTranslation();
  if (state === 'ready') {
    return (
      <Banner role={null} title={t('security.thisDevice.ready.title')} tone="success">
        <p>{t('security.thisDevice.ready.detail')}</p>
      </Banner>
    );
  }
  if (state === 'working') {
    return (
      <Banner
        icon={<LoaderCircle className="security-spin" />}
        title={t('security.thisDevice.working.title')}
        tone="info"
      >
        <p>{t('security.thisDevice.working.detail')}</p>
      </Banner>
    );
  }
  return (
    <Banner
      action={
        <Button
          icon={<RefreshCw aria-hidden="true" />}
          onClick={onRetry}
          size="compact"
          tone="primary"
        >
          {t('security.thisDevice.retry')}
        </Button>
      }
      title={t('security.thisDevice.failed.title')}
      tone="warning"
    >
      <p>{t('security.thisDevice.failed.detail')}</p>
    </Banner>
  );
}
