import { Button, Details } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Bot,
  CircleAlert,
  Clock3,
  LoaderCircle,
  MonitorSmartphone,
  RotateCcw,
  ShieldOff,
} from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  agentInstanceQueryKey,
  productDeviceQueryKey,
  useAgentInstances,
  useProductDevices,
} from '@/features/security/data/access-management-queries';
import {
  deviceAuthorizationExpired,
  groupDevices,
  groupInstances,
} from '@/features/security/domain/access-groups';
import type {
  AccessManagementFailure,
  AccessManagementGateway,
  AgentInstance,
  ProductDevice,
} from '@/features/security/domain/access-management';
import {
  BulkDeviceRevocationDialog,
  DeviceUsage,
  type Reauthentication,
} from '@/features/security/ui/bulk-device-revocation-dialog';

const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

export type AccessManagementLedgerProps = {
  readonly gateway: AccessManagementGateway;
  /** 撤销要最近登录过；不给就不先拦着，服务器说要重新登录时只报错。 */
  readonly reauthentication?: Reauthentication | undefined;
};

type RevocationIntent =
  | { readonly id: string; readonly kind: 'device'; readonly label: string }
  | { readonly id: string; readonly kind: 'instance'; readonly label: string };

export function AccessManagementLedger({ gateway, reauthentication }: AccessManagementLedgerProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const devices = useProductDevices(gateway);
  const instances = useAgentInstances(gateway);
  const [intent, setIntent] = useState<RevocationIntent | null>(null);
  const [bulkOpen, setBulkOpen] = useState(false);
  const [bulkCleanupPending, setBulkCleanupPending] = useState(false);
  const now = Date.now();
  const idleDevices = groupDevices(resultValue(devices.data), now).idle;
  const refresh = async (): Promise<void> => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: productDeviceQueryKey }),
      queryClient.invalidateQueries({ queryKey: agentInstanceQueryKey }),
    ]);
  };
  const revokeDevice = useMutation({
    mutationFn: async (deviceId: string) => await gateway.revokeProductDevice(deviceId),
    onSuccess: async (result) => {
      if (result.ok) {
        setIntent(null);
        await refresh();
      }
    },
  });
  const revokeInstance = useMutation({
    mutationFn: async (instanceId: string) => await gateway.revokeAgentInstance(instanceId),
    onSuccess: async (result) => {
      if (result.ok) {
        setIntent(null);
        await refresh();
      }
    },
  });
  const pendingDeviceCleanup = useMemo(
    () =>
      new Set(
        resultValue(instances.data)
          .filter(
            (instance) =>
              instance.status === 'revoked' && instance.matrixDeviceRevokedAtUnixMs === null,
          )
          .map((instance) => instance.device.deviceId),
      ),
    [instances.data],
  );
  const pending = revokeDevice.isPending || revokeInstance.isPending;
  const failure = resultFailure(revokeDevice.data) ?? resultFailure(revokeInstance.data);

  const confirm = (): void => {
    if (intent?.kind === 'device') {
      revokeDevice.mutate(intent.id);
    } else if (intent?.kind === 'instance') {
      revokeInstance.mutate(intent.id);
    }
  };

  return (
    <section
      aria-labelledby="security-access-title"
      className="security-access"
      id="security-access"
    >
      <header className="security-section-heading security-access__heading">
        <div>
          <h3 id="security-access-title">{t('security.access.title')}</h3>
          <p>{t('security.access.detail')}</p>
        </div>
        <Button
          disabled={devices.isFetching || instances.isFetching}
          icon={<RotateCcw aria-hidden="true" />}
          onClick={() => void refresh()}
          size="compact"
          tone="quiet"
        >
          {t('security.access.refresh')}
        </Button>
      </header>

      <AnimatePresence initial={false}>
        {intent === null ? null : (
          <motion.div
            animate={{ height: 'auto', opacity: 1 }}
            className="security-access__confirmation"
            exit={{ height: 0, opacity: 0 }}
            initial={{ height: 0, opacity: 0 }}
            key={`${intent.kind}:${intent.id}`}
            transition={{ type: 'spring', bounce: 0, duration: 0.35 }}
          >
            <ShieldOff aria-hidden="true" />
            <div>
              <strong>{t(`security.access.confirm.${intent.kind}.title`)}</strong>
              <p>{t(`security.access.confirm.${intent.kind}.detail`, { label: intent.label })}</p>
            </div>
            <div>
              <Button
                disabled={pending}
                onClick={() => {
                  setIntent(null);
                }}
                size="compact"
                tone="quiet"
              >
                {t('security.access.cancel')}
              </Button>
              <Button disabled={pending} onClick={confirm} size="compact" tone="alert">
                {pending ? t('security.access.revoking') : t('security.access.confirm')}
              </Button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {failure === null ? null : (
        <AccessFailure failure={failure} reauthentication={reauthentication} />
      )}
      {bulkCleanupPending || pendingCleanupNotice(revokeDevice.data, revokeInstance.data) ? (
        <p className="security-access__pending" role="status">
          <Clock3 aria-hidden="true" />
          <span>{t('security.access.cleanupPending')}</span>
        </p>
      ) : null}

      <div className="security-access__grid">
        <ProductDevicePanel
          devices={resultValue(devices.data)}
          failure={resultFailure(devices.data)}
          loading={devices.isPending}
          now={now}
          onBulkRevoke={() => {
            setIntent(null);
            setBulkOpen(true);
          }}
          onRetry={() => void devices.refetch()}
          onRevoke={(device) => {
            setIntent({ id: device.deviceId, kind: 'device', label: device.label });
          }}
          pendingCleanup={pendingDeviceCleanup}
          pendingId={revokeDevice.isPending ? revokeDevice.variables : null}
        />
        <AgentInstancePanel
          failure={resultFailure(instances.data)}
          instances={resultValue(instances.data)}
          loading={instances.isPending}
          onRetry={() => void instances.refetch()}
          onRevoke={(instance) => {
            setIntent({
              id: instance.agentInstanceId,
              kind: 'instance',
              label: instance.agentDisplayName,
            });
          }}
          pendingId={revokeInstance.isPending ? revokeInstance.variables : null}
        />
      </div>
      {bulkOpen && idleDevices.length > 0 ? (
        <BulkDeviceRevocationDialog
          devices={idleDevices}
          gateway={gateway}
          now={now}
          onCleanupPending={() => {
            setBulkCleanupPending(true);
          }}
          onClose={() => {
            setBulkOpen(false);
          }}
          onRevoked={refresh}
          reauthentication={reauthentication}
        />
      ) : null}
    </section>
  );
}

type ProductDevicePanelProps = {
  readonly devices: readonly ProductDevice[];
  readonly failure: AccessManagementFailure | null;
  readonly loading: boolean;
  readonly now: number;
  readonly onBulkRevoke: () => void;
  readonly onRetry: () => void;
  readonly onRevoke: (device: ProductDevice) => void;
  readonly pendingCleanup: ReadonlySet<string>;
  readonly pendingId: string | null;
};

/**
 * 最近 7 天用过的摊开；7 天以上没用的、已撤销的收起来、只写有几个（`specs/workspace-tidy/design.md`）。
 * 以前每版发布验收都授权一台新设备，攒下一大片，全摊开就找不到真在用的。
 */
function ProductDevicePanel({
  devices,
  failure,
  loading,
  now,
  onBulkRevoke,
  onRetry,
  onRevoke,
  pendingCleanup,
  pendingId,
}: ProductDevicePanelProps) {
  const { t } = useTranslation();
  const groups = groupDevices(devices, now);
  const row = (device: ProductDevice) => (
    <DeviceRow
      device={device}
      key={device.deviceId}
      now={now}
      onRevoke={onRevoke}
      pendingId={pendingId}
      retryCleanup={pendingCleanup.has(device.deviceId)}
    />
  );
  return (
    <article className="security-access-panel">
      <AccessPanelHeading
        // 只数还登录着的：撤销了的、授权已经过期的都不算。
        count={
          groups.recent.length +
          groups.idle.filter((device) => !deviceAuthorizationExpired(device, now)).length
        }
        detail={t('security.access.devices.detail')}
        icon={<MonitorSmartphone aria-hidden="true" />}
        title={t('security.access.devices.title')}
      />
      <AccessPanelBoundary failure={failure} loading={loading} onRetry={onRetry}>
        {devices.length === 0 ? (
          <p className="security-access-panel__empty">{t('security.access.devices.empty')}</p>
        ) : (
          <div className="security-access-groups">
            {groups.recent.length === 0 ? (
              <p className="security-access-groups__empty">
                {t('security.access.devices.noneRecent')}
              </p>
            ) : (
              <ol className="security-access-list">{groups.recent.map(row)}</ol>
            )}
            {groups.idle.length === 0 ? null : (
              <Details
                className="security-access-group"
                summary={t('security.access.group.idle', { count: groups.idle.length })}
              >
                <div className="security-access-group__actions">
                  <Button
                    disabled={pendingId !== null}
                    onClick={onBulkRevoke}
                    size="compact"
                    tone="alert"
                  >
                    {t('security.access.bulk.open')}
                  </Button>
                </div>
                <ol className="security-access-list">{groups.idle.map(row)}</ol>
              </Details>
            )}
            {groups.revoked.length === 0 ? null : (
              <Details
                className="security-access-group"
                // 有没收完尾、要重试的，展开着让人看到重试按钮。
                defaultOpen={groups.revoked.some((device) => pendingCleanup.has(device.deviceId))}
                summary={t('security.access.group.revoked', { count: groups.revoked.length })}
              >
                <ol className="security-access-list">{groups.revoked.map(row)}</ol>
              </Details>
            )}
          </div>
        )}
      </AccessPanelBoundary>
    </article>
  );
}

function DeviceRow({
  device,
  now,
  onRevoke,
  pendingId,
  retryCleanup,
}: {
  readonly device: ProductDevice;
  readonly now: number;
  readonly onRevoke: (device: ProductDevice) => void;
  readonly pendingId: string | null;
  readonly retryCleanup: boolean;
}) {
  const { t } = useTranslation();
  const state = deviceAuthorizationExpired(device, now) ? 'expired' : device.trustState;
  return (
    <li>
      <div className="security-access-list__identity">
        <strong>{device.label}</strong>
      </div>
      <span className={`security-access-status is-${state}`}>
        {t(`security.access.deviceState.${state}`)}
      </span>
      <DeviceUsage device={device} explainExpiry={false} now={now} />
      {device.trustState !== 'revoked' || retryCleanup ? (
        <Button
          disabled={pendingId !== null}
          onClick={() => {
            onRevoke(device);
          }}
          size="compact"
          tone={retryCleanup ? 'ghost' : 'alert'}
        >
          {pendingId === device.deviceId
            ? t('security.access.revoking')
            : t(retryCleanup ? 'security.access.retryCleanup' : 'security.access.revokeDevice')}
        </Button>
      ) : null}
    </li>
  );
}

type AgentInstancePanelProps = {
  readonly failure: AccessManagementFailure | null;
  readonly instances: readonly AgentInstance[];
  readonly loading: boolean;
  readonly onRetry: () => void;
  readonly onRevoke: (instance: AgentInstance) => void;
  readonly pendingId: string | null;
};

/** 连着的摊开；不在线的、已断开的收起来、只写有几个。 */
function AgentInstancePanel({
  failure,
  instances,
  loading,
  onRetry,
  onRevoke,
  pendingId,
}: AgentInstancePanelProps) {
  const { t } = useTranslation();
  const groups = groupInstances(instances);
  const row = (instance: AgentInstance) => (
    <InstanceRow
      instance={instance}
      key={instance.agentInstanceId}
      onRevoke={onRevoke}
      pendingId={pendingId}
    />
  );
  return (
    <article className="security-access-panel">
      <AccessPanelHeading
        count={groups.connected.length}
        detail={t('security.access.instances.detail')}
        icon={<Bot aria-hidden="true" />}
        title={t('security.access.instances.title')}
      />
      <AccessPanelBoundary failure={failure} loading={loading} onRetry={onRetry}>
        {instances.length === 0 ? (
          <p className="security-access-panel__empty">{t('security.access.instances.empty')}</p>
        ) : (
          <div className="security-access-groups">
            {groups.connected.length === 0 ? (
              <p className="security-access-groups__empty">
                {t('security.access.instances.noneConnected')}
              </p>
            ) : (
              <ol className="security-access-list">{groups.connected.map(row)}</ol>
            )}
            {groups.offline.length === 0 ? null : (
              <Details
                className="security-access-group"
                summary={t('security.access.group.offline', { count: groups.offline.length })}
              >
                <ol className="security-access-list">{groups.offline.map(row)}</ol>
              </Details>
            )}
            {groups.revoked.length === 0 ? null : (
              <Details
                className="security-access-group"
                defaultOpen={groups.revoked.some(needsCleanupRetry)}
                summary={t('security.access.group.disconnected', {
                  count: groups.revoked.length,
                })}
              >
                <ol className="security-access-list">{groups.revoked.map(row)}</ol>
              </Details>
            )}
          </div>
        )}
      </AccessPanelBoundary>
    </article>
  );
}

function InstanceRow({
  instance,
  onRevoke,
  pendingId,
}: {
  readonly instance: AgentInstance;
  readonly onRevoke: (instance: AgentInstance) => void;
  readonly pendingId: string | null;
}) {
  const { t } = useTranslation();
  const retryCleanup = needsCleanupRetry(instance);
  return (
    <li>
      <div className="security-access-list__identity">
        <strong>{instance.agentDisplayName}</strong>
      </div>
      <span className={`security-access-status is-${instance.status}`}>
        {t(`security.access.instanceState.${instance.status}`)}
      </span>
      <AccessMetadata
        primary={instance.device.label}
        timestamp={instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs}
        timestampKind={instance.lastSeenAtUnixMs === null ? 'registered' : 'lastSeen'}
      />
      {instance.status !== 'revoked' || retryCleanup ? (
        <Button
          disabled={pendingId !== null}
          onClick={() => {
            onRevoke(instance);
          }}
          size="compact"
          tone={retryCleanup ? 'ghost' : 'alert'}
        >
          {pendingId === instance.agentInstanceId
            ? t('security.access.revoking')
            : t(retryCleanup ? 'security.access.retryCleanup' : 'security.access.revokeInstance')}
        </Button>
      ) : null}
    </li>
  );
}

function needsCleanupRetry(instance: AgentInstance): boolean {
  return instance.status === 'revoked' && instance.matrixDeviceRevokedAtUnixMs === null;
}

type AccessPanelHeadingProps = {
  readonly count: number;
  readonly detail: string;
  readonly icon: React.ReactNode;
  readonly title: string;
};

function AccessPanelHeading({ count, detail, icon, title }: AccessPanelHeadingProps) {
  return (
    <header className="security-access-panel__heading">
      <div className="security-access-panel__icon">{icon}</div>
      <div>
        <h4>{title}</h4>
        <p>{detail}</p>
      </div>
      <span>{count.toString().padStart(2, '0')}</span>
    </header>
  );
}

type AccessPanelBoundaryProps = React.PropsWithChildren<{
  readonly failure: AccessManagementFailure | null;
  readonly loading: boolean;
  readonly onRetry: () => void;
}>;

function AccessPanelBoundary({ children, failure, loading, onRetry }: AccessPanelBoundaryProps) {
  const { t } = useTranslation();
  if (loading) {
    return (
      <p className="security-access-panel__boundary" role="status">
        <LoaderCircle aria-hidden="true" className="security-spin" />
        {t('security.access.loading')}
      </p>
    );
  }
  if (failure !== null) {
    return (
      <div className="security-access-panel__boundary" role="alert">
        <CircleAlert aria-hidden="true" />
        <span>{t('security.access.loadFailed')}</span>
        <Button onClick={onRetry} size="compact" tone="quiet">
          {t('security.access.retry')}
        </Button>
      </div>
    );
  }
  return children;
}

function AccessMetadata({
  primary,
  timestamp,
  timestampKind,
}: {
  readonly primary: string;
  readonly timestamp: number;
  readonly timestampKind: 'lastSeen' | 'registered';
}) {
  const { i18n, t } = useTranslation();
  return (
    <div className="security-access-list__metadata">
      <span>{primary}</span>
      <time dateTime={new Date(timestamp).toISOString()}>
        {t(`security.access.${timestampKind}`, {
          time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
            dateStyle: 'medium',
            timeStyle: 'short',
          }).format(timestamp),
        })}
      </time>
    </div>
  );
}

function AccessFailure({
  failure,
  reauthentication,
}: {
  readonly failure: AccessManagementFailure;
  readonly reauthentication: Reauthentication | undefined;
}) {
  const { t } = useTranslation();
  if (failure.code === REAUTHENTICATION_REQUIRED && reauthentication !== undefined) {
    return (
      <p className="security-failure" role="alert">
        <CircleAlert aria-hidden="true" />
        <span>{t('security.access.signInAgain')}</span>
        <Button onClick={reauthentication.signInAgain} size="compact" tone="quiet">
          {t('security.access.bulk.signInAgain')}
        </Button>
      </p>
    );
  }
  return (
    <p className="security-failure" role="alert">
      <CircleAlert aria-hidden="true" />
      <span>
        {t('security.access.actionFailed')} <code>{failure.code}</code>
        {failure.correlationId === undefined ? null : ` · ${failure.correlationId}`}
      </span>
    </p>
  );
}

function resultValue<T>(
  result: { readonly ok: true; readonly value: readonly T[] } | { readonly ok: false } | undefined,
): readonly T[] {
  return result?.ok === true ? result.value : [];
}

function resultFailure(
  result:
    | { readonly error: AccessManagementFailure; readonly ok: false }
    | { readonly ok: true }
    | undefined,
): AccessManagementFailure | null {
  return result?.ok === false ? result.error : null;
}

function pendingCleanupNotice(
  device:
    | { readonly ok: true; readonly value: { readonly matrixCleanup: string } }
    | { readonly ok: false }
    | undefined,
  instance:
    | { readonly ok: true; readonly value: { readonly matrixCleanup: string } }
    | { readonly ok: false }
    | undefined,
): boolean {
  return (
    (device?.ok === true && device.value.matrixCleanup === 'pending') ||
    (instance?.ok === true && instance.value.matrixCleanup === 'pending')
  );
}
