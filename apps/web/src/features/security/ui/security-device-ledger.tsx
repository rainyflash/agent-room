import { Button } from '@agent-room/ui-system';
import { Fingerprint, Laptop, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { MatrixSecurityDevice } from '@/features/security/domain/matrix-security';
import { trustMessageKey } from '@/features/security/ui/security-copy';

export type SecurityDeviceLedgerProps = {
  readonly devices: readonly MatrixSecurityDevice[];
  readonly onVerify: (device: MatrixSecurityDevice) => void;
  readonly pendingDeviceId: string | null;
  readonly verificationAvailable: boolean;
  readonly verificationOpen: boolean;
};

export function SecurityDeviceLedger({
  devices,
  onVerify,
  pendingDeviceId,
  verificationAvailable,
  verificationOpen,
}: SecurityDeviceLedgerProps) {
  const { t } = useTranslation();
  const orderedDevices = [...devices].sort(
    (left, right) => Number(right.current) - Number(left.current),
  );

  return (
    <section aria-labelledby="security-devices-title" className="security-devices">
      <header className="security-section-heading">
        <div>
          <h2 id="security-devices-title">{t('security.devices.title')}</h2>
          <p>{t('security.devices.detail')}</p>
        </div>
        <span>{devices.length.toString().padStart(2, '0')}</span>
      </header>
      {orderedDevices.length === 0 ? (
        <p className="security-devices__empty">{t('security.devices.empty')}</p>
      ) : (
        <ol className="security-devices__list">
          {orderedDevices.map((device) => {
            const verifiable = verificationAvailable && needsVerification(device);
            const pending = pendingDeviceId === device.deviceId;
            return (
              <li className={device.current ? 'is-current' : undefined} key={device.deviceId}>
                <div className="security-device__icon">
                  <Laptop aria-hidden="true" />
                </div>
                <div className="security-device__identity">
                  <span>
                    {t(device.current ? 'security.devices.current' : 'security.devices.other')}
                  </span>
                  <strong>{device.displayName ?? t('security.devices.unnamed')}</strong>
                  <small>{device.deviceId}</small>
                </div>
                <div className="security-device__fingerprint">
                  <Fingerprint aria-hidden="true" />
                  <span>{device.fingerprint ?? t('security.devices.fingerprintMissing')}</span>
                </div>
                <div className={`security-device__trust security-device__trust--${device.trust}`}>
                  <ShieldCheck aria-hidden="true" />
                  <span>{t(trustMessageKey[device.trust])}</span>
                </div>
                {verifiable ? (
                  <Button
                    disabled={verificationOpen}
                    onClick={() => {
                      onVerify(device);
                    }}
                    size="compact"
                    tone="ghost"
                  >
                    {pending ? `${t('security.devices.verify')}…` : t('security.devices.verify')}
                  </Button>
                ) : null}
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}

function needsVerification(device: MatrixSecurityDevice): boolean {
  return device.current
    ? device.trust !== 'verified'
    : device.trust !== 'verified' && device.trust !== 'signed';
}
