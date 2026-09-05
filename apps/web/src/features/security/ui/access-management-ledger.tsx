import { Button } from '@agent-room/ui-system';
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
import type {
  AccessManagementFailure,
  AccessManagementGateway,
  AgentInstance,
  ProductDevice,
} from '@/features/security/domain/access-management';

export type AccessManagementLedgerProps = {
  readonly gateway: AccessManagementGateway;
};

type RevocationIntent =
  | { readonly id: string; readonly kind: 'device'; readonly label: string }
  | { readonly id: string; readonly kind: 'instance'; readonly label: string };

export function AccessManagementLedger({ gateway }: AccessManagementLedgerProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const devices = useProductDevices(gateway);
  const instances = useAgentInstances(gateway);
  const [intent, setIntent] = useState<RevocationIntent | null>(null);
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
    <section aria-labelledby="security-access-title" className="security-access">
      <header className="security-section-heading security-access__heading">
        <div>
          <h2 id="security-access-title">{t('security.access.title')}</h2>
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

      {failure === null ? null : <AccessFailure failure={failure} />}
      {pendingCleanupNotice(revokeDevice.data, revokeInstance.data) ? (
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
    </section>
  );
}

type ProductDevicePanelProps = {
  readonly devices: readonly ProductDevice[];
  readonly failure: AccessManagementFailure | null;
  readonly loading: boolean;
  readonly onRetry: () => void;
  readonly onRevoke: (device: ProductDevice) => void;
  readonly pendingCleanup: ReadonlySet<string>;
  readonly pendingId: string | null;
};

function ProductDevicePanel({
  devices,
  failure,
  loading,
  onRetry,
  onRevoke,
  pendingCleanup,
  pendingId,
}: ProductDevicePanelProps) {
  const { t } = useTranslation();
  return (
    <article className="security-access-panel">
      <AccessPanelHeading
        count={devices.length}
        detail={t('security.access.devices.detail')}
        icon={<MonitorSmartphone aria-hidden="true" />}
        title={t('security.access.devices.title')}
      />
      <AccessPanelBoundary failure={failure} loading={loading} onRetry={onRetry}>
        {devices.length === 0 ? (
          <p className="security-access-panel__empty">{t('security.access.devices.empty')}</p>
        ) : (
          <ol className="security-access-list">
            {sortByRevocation(devices).map((device) => {
              const retryCleanup = pendingCleanup.has(device.deviceId);
              return (
                <li key={device.deviceId}>
                  <div className="security-access-list__identity">
                    <strong>{device.label}</strong>
                    <small>{device.deviceId}</small>
                  </div>
                  <span className={`security-access-status is-${device.trustState}`}>
                    {t(`security.access.deviceState.${device.trustState}`)}
                  </span>
                  <AccessMetadata
                    primary={t(`security.access.platform.${device.platform}`)}
                    timestamp={device.lastSeenAtUnixMs ?? device.createdAtUnixMs}
                    timestampKind={device.lastSeenAtUnixMs === null ? 'registered' : 'lastSeen'}
                  />
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
                        : t(
                            retryCleanup
                              ? 'security.access.retryCleanup'
                              : 'security.access.revokeDevice',
                          )}
                    </Button>
                  ) : null}
                </li>
              );
            })}
          </ol>
        )}
      </AccessPanelBoundary>
    </article>
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

function AgentInstancePanel({
  failure,
  instances,
  loading,
  onRetry,
  onRevoke,
  pendingId,
}: AgentInstancePanelProps) {
  const { t } = useTranslation();
  return (
    <article className="security-access-panel">
      <AccessPanelHeading
        count={instances.length}
        detail={t('security.access.instances.detail')}
        icon={<Bot aria-hidden="true" />}
        title={t('security.access.instances.title')}
      />
      <AccessPanelBoundary failure={failure} loading={loading} onRetry={onRetry}>
        {instances.length === 0 ? (
          <p className="security-access-panel__empty">{t('security.access.instances.empty')}</p>
        ) : (
          <ol className="security-access-list">
            {sortByRevocation(instances).map((instance) => {
              const retryCleanup =
                instance.status === 'revoked' && instance.matrixDeviceRevokedAtUnixMs === null;
              return (
                <li key={instance.agentInstanceId}>
                  <div className="security-access-list__identity">
                    <strong>{instance.agentDisplayName}</strong>
                    <small>
                      {instance.adapterType} · {instance.matrixDeviceId}
                    </small>
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
                        : t(
                            retryCleanup
                              ? 'security.access.retryCleanup'
                              : 'security.access.revokeInstance',
                          )}
                    </Button>
                  ) : null}
                </li>
              );
            })}
          </ol>
        )}
      </AccessPanelBoundary>
    </article>
  );
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
        <h3>{title}</h3>
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

function AccessFailure({ failure }: { readonly failure: AccessManagementFailure }) {
  const { t } = useTranslation();
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

function sortByRevocation<T extends { readonly revokedAtUnixMs: number | null }>(
  values: readonly T[],
): readonly T[] {
  return [...values].sort(
    (left, right) => Number(left.revokedAtUnixMs !== null) - Number(right.revokedAtUnixMs !== null),
  );
}
