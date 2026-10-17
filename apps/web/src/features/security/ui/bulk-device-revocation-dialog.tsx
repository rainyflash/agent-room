import { Banner, Button, Dialog } from '@agent-room/ui-system';
import { ShieldOff } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  deviceAuthorizationExpired,
  deviceLastUsedAt,
} from '@/features/security/domain/access-groups';
import type {
  AccessManagementGateway,
  ProductDevice,
} from '@/features/security/domain/access-management';

const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

/** 撤销电脑要最近登录过；没有的话先请人重新登录一次。 */
export type Reauthentication = {
  readonly recentlyAuthenticated: boolean;
  readonly signInAgain: () => void;
};

type Progress = { readonly done: number; readonly total: number };

/**
 * “7 天以上没用的”一键撤销（维护者 2026-10-17 定）：列出这一组的每台电脑，默认都勾上，
 * 要留着的取消勾选，确认后一台台撤销。没撤成的留在列表里、还勾着，可以再点一次。
 */
export function BulkDeviceRevocationDialog({
  devices,
  gateway,
  now,
  onClose,
  onCleanupPending,
  onRevoked,
  reauthentication,
}: {
  readonly devices: readonly ProductDevice[];
  readonly gateway: AccessManagementGateway;
  readonly now: number;
  readonly onClose: () => void;
  readonly onCleanupPending: () => void;
  readonly onRevoked: () => Promise<void>;
  readonly reauthentication?: Reauthentication | undefined;
}) {
  const { t } = useTranslation();
  const [kept, setKept] = useState<ReadonlySet<string>>(() => new Set());
  const [progress, setProgress] = useState<Progress | null>(null);
  const [failed, setFailed] = useState(0);
  const [signInRejected, setSignInRejected] = useState(false);
  const needsSignIn =
    signInRejected || (reauthentication !== undefined && !reauthentication.recentlyAuthenticated);
  const chosen = devices.filter((device) => !kept.has(device.deviceId));
  const running = progress !== null;

  const revoke = async (): Promise<void> => {
    const targets = chosen;
    let failures = 0;
    let rejected = false;
    setFailed(0);
    setProgress({ done: 0, total: targets.length });
    for (const [index, device] of targets.entries()) {
      const result = await gateway.revokeProductDevice(device.deviceId);
      if (!result.ok && result.error.code === REAUTHENTICATION_REQUIRED) {
        rejected = true;
        break;
      }
      if (!result.ok) failures += 1;
      else if (result.value.matrixCleanup === 'pending') onCleanupPending();
      setProgress({ done: index + 1, total: targets.length });
    }
    await onRevoked();
    setProgress(null);
    setFailed(failures);
    setSignInRejected(rejected);
    if (failures === 0 && !rejected) onClose();
  };

  return (
    <Dialog
      className="security-bulk-revocation"
      closeLabel={t('security.access.bulk.close')}
      description={t('security.access.bulk.detail')}
      footer={
        <>
          <Button disabled={running} onClick={onClose} size="compact" tone="quiet">
            {t('security.access.cancel')}
          </Button>
          {needsSignIn && reauthentication !== undefined ? (
            <Button onClick={reauthentication.signInAgain} size="compact" tone="primary">
              {t('security.access.bulk.signInAgain')}
            </Button>
          ) : (
            <Button
              disabled={running || chosen.length === 0}
              onClick={() => void revoke()}
              size="compact"
              tone="alert"
            >
              {progress === null
                ? t('security.access.bulk.confirm', { count: chosen.length })
                : t('security.access.bulk.progress', progress)}
            </Button>
          )}
        </>
      }
      icon={<ShieldOff />}
      onClose={() => {
        if (!running) onClose();
      }}
      title={t('security.access.bulk.title')}
    >
      {needsSignIn ? (
        <Banner icon={null} role="status" title={t('security.access.bulk.signIn')} tone="info" />
      ) : null}
      {failed > 0 ? (
        <p className="security-failure" role="alert">
          {t('security.access.bulk.failed', { count: failed })}
        </p>
      ) : null}
      <ul className="security-bulk-revocation__list">
        {devices.map((device) => (
          <li key={device.deviceId}>
            <label>
              <input
                checked={!kept.has(device.deviceId)}
                disabled={running}
                onChange={(event) => {
                  const keep = !event.target.checked;
                  setKept((current) => {
                    const next = new Set(current);
                    if (keep) next.add(device.deviceId);
                    else next.delete(device.deviceId);
                    return next;
                  });
                }}
                type="checkbox"
              />
              <span className="security-bulk-revocation__identity">
                <strong>{device.label}</strong>
                <DeviceUsage device={device} now={now} />
              </span>
            </label>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}

/** 平台、上次使用，过期了的再说一句要重新授权（设备列表的状态里已经写着时不再说）。 */
export function DeviceUsage({
  device,
  now,
  explainExpiry = true,
}: {
  readonly device: ProductDevice;
  readonly now: number;
  readonly explainExpiry?: boolean;
}) {
  const { i18n, t } = useTranslation();
  const usedAt = deviceLastUsedAt(device);
  return (
    <span className="security-access-list__metadata">
      <span>{t(`security.access.platform.${device.platform}`)}</span>
      <time dateTime={new Date(usedAt).toISOString()}>
        {t(
          device.lastSeenAtUnixMs === null
            ? 'security.access.registered'
            : 'security.access.lastUsed',
          {
            time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
              dateStyle: 'medium',
              timeStyle: 'short',
            }).format(usedAt),
          },
        )}
      </time>
      {explainExpiry && deviceAuthorizationExpired(device, now) ? (
        <span className="security-access-list__expired">{t('security.access.expired')}</span>
      ) : null}
    </span>
  );
}
