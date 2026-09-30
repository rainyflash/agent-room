import { Banner, Button } from '@agent-room/ui-system';
import { KeyRound, MonitorSmartphone, ShieldPlus } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { ThisDeviceSigning as SigningState } from '@/features/security/domain/device-signing';

export type ThisDeviceSigningProps = {
  readonly state: SigningState;
  readonly identityPending: boolean;
  readonly verificationPending: boolean;
  /** 有别的已登录设备时才能“用另一台设备核对”。 */
  readonly otherDevices: boolean;
  readonly onEstablishIdentity: () => void;
  readonly onUseRecoveryKey: () => void;
  readonly onVerifyWithOtherDevice: () => void;
};

/**
 * 这台设备：已由你签名，或者需要签名。需要签名时给两条路——恢复密钥、另一台已登录的设备，
 * 结果一样：这台设备由你签名，加密房间就能用了。
 */
export function ThisDeviceSigning({
  state,
  identityPending,
  verificationPending,
  otherDevices,
  onEstablishIdentity,
  onUseRecoveryKey,
  onVerifyWithOtherDevice,
}: ThisDeviceSigningProps) {
  const { t } = useTranslation();
  if (state === 'signed') {
    return (
      <Banner role={null} title={t('security.signing.signed.title')} tone="success">
        <p>{t('security.signing.signed.detail')}</p>
      </Banner>
    );
  }
  if (state === 'needs_identity') {
    return (
      <Banner
        action={
          <Button
            disabled={identityPending}
            icon={<ShieldPlus aria-hidden="true" />}
            onClick={onEstablishIdentity}
            size="compact"
            tone="primary"
          >
            {t('security.identity.establish')}
          </Button>
        }
        title={t('security.signing.identity.title')}
        tone="warning"
      >
        <p>{t('security.signing.identity.detail')}</p>
      </Banner>
    );
  }
  return (
    <section aria-label={t('security.signing.needed.title')} className="security-signing">
      <Banner role={null} title={t('security.signing.needed.title')} tone="warning">
        <p>{t('security.signing.needed.detail')}</p>
      </Banner>
      <div className="security-signing__ways">
        <article>
          <KeyRound aria-hidden="true" />
          <h3>{t('security.signing.recovery.title')}</h3>
          <p>{t('security.signing.recovery.detail')}</p>
          <Button onClick={onUseRecoveryKey} size="compact" tone="primary">
            {t('security.signing.recovery.action')}
          </Button>
        </article>
        <article>
          <MonitorSmartphone aria-hidden="true" />
          <h3>{t('security.signing.device.title')}</h3>
          <p>
            {t(otherDevices ? 'security.signing.device.detail' : 'security.signing.device.none')}
          </p>
          <Button
            disabled={!otherDevices || verificationPending}
            onClick={onVerifyWithOtherDevice}
            size="compact"
            tone="ghost"
          >
            {t('security.signing.device.action')}
          </Button>
        </article>
      </div>
      <p className="security-signing__same">{t('security.signing.same')}</p>
    </section>
  );
}
