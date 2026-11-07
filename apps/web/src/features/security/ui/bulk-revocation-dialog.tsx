import { Banner, Button, Dialog } from '@agent-room/ui-system';
import { ShieldOff } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  deviceAuthorizationExpired,
  deviceLastUsedAt,
} from '@/features/security/domain/access-groups';
import type {
  AccessManagementFailure,
  AccessManagementGateway,
  AgentInstance,
  ProductDevice,
} from '@/features/security/domain/access-management';
import type { Result } from '@/shared/result';

const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

/**
 * 服务器说要重新登录时的去处。撤销本身不要最近登录过（`specs/cleanup-without-reauth/design.md`），
 * 只有回滚到旧版控制面时服务器才会这样拒。
 */
export type Reauthentication = {
  readonly signInAgain: () => void;
};

/**
 * 一次撤几个：
 * - `idleDevices`：“7 天以上没用的”一键撤销（维护者 2026-10-17 定），默认都勾上，要留着的取消勾选；
 * - `devices`、`instances`：在还登录着的电脑、还没断开的 Agent 连接里自己挑（维护者 2026-11-07 定），
 *   默认都不勾。
 */
export type BulkRevocation =
  | { readonly kind: 'devices' | 'idleDevices'; readonly devices: readonly ProductDevice[] }
  | { readonly kind: 'instances'; readonly instances: readonly AgentInstance[] };

type Target = { readonly id: string; readonly label: string; readonly usage: ReactNode };

type Progress = { readonly done: number; readonly total: number };

type RevocationResult = Result<
  { readonly matrixCleanup: 'complete' | 'pending' },
  AccessManagementFailure
>;

/**
 * 勾选以后一个个撤。没撤成的留在列表里、还勾着，可以再点一次；都撤成了就关上。
 * 服务器说要重新登录时停下，改成请人重新登录。
 */
export function BulkRevocationDialog({
  gateway,
  now,
  onClose,
  onCleanupPending,
  onRevoked,
  reauthentication,
  revocation,
}: {
  readonly gateway: AccessManagementGateway;
  readonly now: number;
  readonly onClose: () => void;
  readonly onCleanupPending: () => void;
  readonly onRevoked: () => Promise<void>;
  readonly reauthentication?: Reauthentication | undefined;
  readonly revocation: BulkRevocation;
}) {
  const { t } = useTranslation();
  const targets = bulkTargets(revocation, now);
  const instances = revocation.kind === 'instances';
  const [chosen, setChosen] = useState<ReadonlySet<string>>(
    () => new Set(revocation.kind === 'idleDevices' ? targets.map((target) => target.id) : []),
  );
  const [progress, setProgress] = useState<Progress | null>(null);
  const [failed, setFailed] = useState(0);
  const [signInRejected, setSignInRejected] = useState(false);
  const selected = targets.filter((target) => chosen.has(target.id));
  const running = progress !== null;
  const everything = selected.length === targets.length;
  const count = selected.length;
  // 撤着的时候写进度；一个都没勾时只写“撤销”“断开”，不写“撤销 0 台”。
  const confirmLabel =
    progress !== null
      ? instances
        ? t('security.access.bulk.progressInstances', progress)
        : t('security.access.bulk.progress', progress)
      : count === 0
        ? instances
          ? t('security.access.revokeInstance')
          : t('security.access.revokeDevice')
        : instances
          ? t('security.access.bulk.confirmInstances', { count })
          : t('security.access.bulk.confirm', { count });

  const revokeOne = async (id: string): Promise<RevocationResult> =>
    instances ? await gateway.revokeAgentInstance(id) : await gateway.revokeProductDevice(id);

  const revoke = async (): Promise<void> => {
    const queue = selected;
    const revoked = new Set<string>();
    let failures = 0;
    let rejected = false;
    setFailed(0);
    setProgress({ done: 0, total: queue.length });
    for (const [index, target] of queue.entries()) {
      const result = await revokeOne(target.id);
      if (!result.ok && result.error.code === REAUTHENTICATION_REQUIRED) {
        rejected = true;
        break;
      }
      if (result.ok) {
        revoked.add(target.id);
        if (result.value.matrixCleanup === 'pending') onCleanupPending();
      } else {
        failures += 1;
      }
      setProgress({ done: index + 1, total: queue.length });
    }
    await onRevoked();
    setChosen((current) => new Set([...current].filter((id) => !revoked.has(id))));
    setProgress(null);
    setFailed(failures);
    setSignInRejected(rejected);
    if (failures === 0 && !rejected) onClose();
  };

  const toggle = (id: string, on: boolean): void => {
    setChosen((current) => {
      const next = new Set(current);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  return (
    <Dialog
      className="security-bulk-revocation"
      closeLabel={t('security.access.bulk.close')}
      description={t(`security.access.bulk.${revocation.kind}.detail`)}
      footer={
        <>
          <Button disabled={running} onClick={onClose} size="compact" tone="quiet">
            {t('security.access.cancel')}
          </Button>
          {signInRejected && reauthentication !== undefined ? (
            <Button onClick={reauthentication.signInAgain} size="compact" tone="primary">
              {t('security.access.bulk.signInAgain')}
            </Button>
          ) : (
            <Button
              disabled={running || selected.length === 0}
              onClick={() => void revoke()}
              size="compact"
              tone="alert"
            >
              {confirmLabel}
            </Button>
          )}
        </>
      }
      icon={<ShieldOff />}
      onClose={() => {
        if (!running) onClose();
      }}
      title={t(`security.access.bulk.${revocation.kind}.title`)}
    >
      {signInRejected ? (
        <Banner icon={null} role="status" title={t('security.access.bulk.signIn')} tone="info" />
      ) : null}
      {failed > 0 ? (
        <p className="security-failure" role="alert">
          {instances
            ? t('security.access.bulk.failedInstances', { count: failed })
            : t('security.access.bulk.failed', { count: failed })}
        </p>
      ) : null}
      {targets.length > 1 ? (
        <div className="security-bulk-revocation__select">
          <Button
            disabled={running}
            onClick={() => {
              setChosen(new Set(everything ? [] : targets.map((target) => target.id)));
            }}
            size="compact"
            tone="quiet"
          >
            {everything
              ? t('security.access.bulk.selectNone')
              : t('security.access.bulk.selectAll')}
          </Button>
        </div>
      ) : null}
      <ul className="security-bulk-revocation__list">
        {targets.map((target) => (
          <li key={target.id}>
            <label>
              <input
                checked={chosen.has(target.id)}
                disabled={running}
                onChange={(event) => {
                  toggle(target.id, event.target.checked);
                }}
                type="checkbox"
              />
              <span className="security-bulk-revocation__identity">
                <strong>{target.label}</strong>
                {target.usage}
              </span>
            </label>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}

function bulkTargets(revocation: BulkRevocation, now: number): readonly Target[] {
  if (revocation.kind === 'instances') {
    return revocation.instances.map((instance) => ({
      id: instance.agentInstanceId,
      label: instance.agentDisplayName,
      usage: <InstanceUsage instance={instance} />,
    }));
  }
  return revocation.devices.map((device) => ({
    id: device.deviceId,
    label: device.label,
    usage: <DeviceUsage device={device} now={now} />,
  }));
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

/** 连在哪台电脑上、现在连没连着、上次活动的时候。 */
function InstanceUsage({ instance }: { readonly instance: AgentInstance }) {
  const { i18n, t } = useTranslation();
  const seenAt = instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs;
  return (
    <span className="security-access-list__metadata">
      <span>{instance.device.label}</span>
      <span>{t(`security.access.instanceState.${instance.status}`)}</span>
      <time dateTime={new Date(seenAt).toISOString()}>
        {t(
          instance.lastSeenAtUnixMs === null
            ? 'security.access.registered'
            : 'security.access.lastSeen',
          {
            time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
              dateStyle: 'medium',
              timeStyle: 'short',
            }).format(seenAt),
          },
        )}
      </time>
    </span>
  );
}
