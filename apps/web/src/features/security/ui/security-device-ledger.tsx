import { Button, Details } from '@agent-room/ui-system';
import { Laptop, ShieldAlert, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { deviceSignedByYou } from '@/features/security/domain/device-signing';
import type { MatrixSecurityDevice } from '@/features/security/domain/matrix-security';

export type SecurityDeviceLedgerProps = {
  readonly devices: readonly MatrixSecurityDevice[];
  /** 这台设备签好没有；它的签名在上面一节处理，这里只列状态。 */
  readonly thisDeviceSigned: boolean;
  readonly onVerify: (device: MatrixSecurityDevice) => void;
  readonly pendingDeviceId: string | null;
  readonly verificationAvailable: boolean;
  readonly verificationOpen: boolean;
};

/**
 * 你的设备：名字、是不是这台、由你签名没有。只有由你签名的设备拿得到加密房间的钥匙；
 * 没签名的别的设备可以在这里核对。指纹和设备 ID 收在详情里。
 */
export function SecurityDeviceLedger({
  devices,
  thisDeviceSigned,
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
          <h3 id="security-devices-title">{t('security.devices.title')}</h3>
          <p>{t('security.devices.detail')}</p>
        </div>
      </header>
      {orderedDevices.length === 0 ? (
        <p className="security-devices__empty">{t('security.devices.empty')}</p>
      ) : (
        <ol className="security-devices__list">
          {orderedDevices.map((device) => {
            const signed = device.current ? thisDeviceSigned : deviceSignedByYou(device);
            const verifiable = verificationAvailable && !device.current && !signed;
            const pending = pendingDeviceId === device.deviceId;
            return (
              <li className={device.current ? 'is-current' : undefined} key={device.deviceId}>
                <div className="security-device__icon">
                  <Laptop aria-hidden="true" />
                </div>
                <div className="security-device__identity">
                  <strong>{device.displayName ?? t('security.devices.unnamed')}</strong>
                  {device.current ? (
                    <span className="security-device__tag">{t('security.devices.current')}</span>
                  ) : null}
                </div>
                <div
                  className={`security-device__trust security-device__trust--${signed ? 'signed' : 'unsigned'}`}
                >
                  {signed ? <ShieldCheck aria-hidden="true" /> : <ShieldAlert aria-hidden="true" />}
                  <span>
                    {t(signed ? 'security.devices.signed' : 'security.devices.notSigned')}
                  </span>
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
                <Details className="security-device__details" summary={t('security.details')}>
                  <dl>
                    <div>
                      <dt>{t('security.devices.deviceId')}</dt>
                      <dd>
                        <code>{device.deviceId}</code>
                      </dd>
                    </div>
                    <div>
                      <dt>{t('security.devices.fingerprint')}</dt>
                      <dd>
                        <code>
                          {device.fingerprint ?? t('security.devices.fingerprintMissing')}
                        </code>
                      </dd>
                    </div>
                  </dl>
                </Details>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
