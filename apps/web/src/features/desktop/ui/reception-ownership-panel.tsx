import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@agent-room/ui-system';
import { useAppServices } from '@/app/app-services';
import { useSession } from '@/features/session/ui/session-provider';
import type { ProductDevice } from '@/features/security/domain/access-management';
import { err } from '@/shared/result';
import { encodeCliInvitation } from '../domain/cli-invitation';
import { receptionAvailability, type ReceptionRecord } from '../domain/reception-ownership';
import './reception-panel.css';

type OwnershipProps = {
  readonly agentId?: string | undefined;
  readonly roomId?: string | undefined;
  /** Inside the reception panel, whose refresh button already covers this list. */
  readonly embedded?: boolean;
};
export const receptionOwnershipQueryKeys = {
  records: (principalId: string | null) => ['reception-ownership', principalId] as const,
  devices: (principalId: string | null) => ['reception-devices', principalId] as const,
};
export function ReceptionOwnershipPanel(props: OwnershipProps) {
  const { receptionOwnership } = useAppServices();
  return receptionOwnership ? <ConnectedOwnershipPanel {...props} /> : null;
}
function ConnectedOwnershipPanel({ agentId, roomId, embedded = false }: OwnershipProps) {
  const { receptionOwnership: gateway, accessManagement } = useAppServices();
  const { snapshot } = useSession();
  const { t } = useTranslation();
  const principal = snapshot.context.principal;
  const records = useQuery({
    queryKey: receptionOwnershipQueryKeys.records(principal?.principalId ?? null),
    queryFn: () => gateway?.list() ?? err({ code: 'reception.unavailable', retryable: false }),
    enabled: gateway !== undefined && principal !== null,
    refetchInterval: 5000,
  });
  const devices = useQuery({
    queryKey: receptionOwnershipQueryKeys.devices(principal?.principalId ?? null),
    queryFn: () => accessManagement.listProductDevices(),
    enabled: gateway !== undefined && principal !== null,
    staleTime: 15000,
  });
  if (!principal || !gateway) return null;
  const rows = records.data?.ok
    ? records.data.value.receptions.filter(
        (record) =>
          (agentId === undefined || record.agentId === agentId) &&
          (roomId === undefined || record.roomId === roomId),
      )
    : [];
  if (!records.isError && records.data?.ok !== false && rows.length === 0) return null;
  return (
    <section
      className="reception-owners"
      data-embedded={embedded ? 'true' : 'false'}
      aria-label={t('receptionOwner.title')}
    >
      <div className="reception-section-head">
        <h3>{t('receptionOwner.title')}</h3>
        {embedded ? null : (
          <button
            type="button"
            className="ar-icon-button reception-refresh"
            aria-label={t('receptionOwner.refresh')}
            title={t('receptionOwner.refresh')}
            onClick={() => {
              void records.refetch();
              void devices.refetch();
            }}
          >
            <RefreshCw aria-hidden="true" />
          </button>
        )}
      </div>
      {records.isError || records.data?.ok === false ? (
        <p role="alert">{t('receptionOwner.failed')}</p>
      ) : null}
      {devices.isError || devices.data?.ok === false ? (
        <p role="alert">{t('receptionOwner.devicesFailed')}</p>
      ) : null}
      {records.data?.ok && records.data.value.limited ? (
        <p className="reception-note">{t('receptionOwner.limited')}</p>
      ) : null}
      {rows.map((record) => (
        <OwnershipCard
          key={`${principal.principalId}:${record.agentId}:${record.catalogId}`}
          record={record}
          principalId={principal.principalId}
          devices={devices.data?.ok ? devices.data.value : []}
          refresh={() => {
            void records.refetch();
          }}
        />
      ))}
    </section>
  );
}
function OwnershipCard({
  record,
  principalId,
  devices,
  refresh,
}: {
  readonly record: ReceptionRecord;
  readonly principalId: string;
  readonly devices: readonly ProductDevice[];
  readonly refresh: () => void;
}) {
  const { t } = useTranslation();
  const { receptionOwnership: gateway, accessManagement } = useAppServices();
  const [target, setTarget] = useState('');
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [cleanupPending, setCleanupPending] = useState(false);
  const transfer = useMutation({
    mutationFn: async ({
      nextDeviceId,
      recover = false,
    }: {
      nextDeviceId: string | null;
      recover?: boolean;
    }) => {
      if (recover) {
        const revoked = await accessManagement.revokeAgentInstance(record.instanceId);
        if (!revoked.ok) return revoked;
        if (revoked.value.matrixCleanup === 'pending') {
          setCleanupPending(true);
          return err({ code: 'reception.cleanup_pending', retryable: true });
        }
        setCleanupPending(false);
      }
      return (
        gateway?.transfer({ agentId: record.agentId, catalogId: record.catalogId, nextDeviceId }) ??
        err({ code: 'reception.unavailable', retryable: false })
      );
    },
    onSettled: refresh,
  });
  const options = devices.filter(
    (device) =>
      device.trustState === 'verified' &&
      device.revokedAtUnixMs === null &&
      device.platform !== 'web',
  );
  const selected = options.find((device) => device.deviceId === target);
  const availability = receptionAvailability(record, Date.now());
  const copy = async () => {
    const invitation = encodeCliInvitation(
      { sessionKey: record.sessionKey, displayName: record.displayName, ownerId: principalId },
      record.roomId,
      record.catalogId,
    );
    try {
      await navigator.clipboard.writeText(
        t('receptionOwner.prompt', { command: `agent-room join --invite ${invitation}` }),
      );
      setCopied(true);
      setCopyFailed(false);
    } catch {
      setCopyFailed(true);
    }
  };
  return (
    <article className="reception-card" data-status={availability}>
      <strong className="reception-card__name">{record.displayName}</strong>
      <div className="reception-card__facts">
        <span className="reception-chip" data-tone={availability}>
          {t(`receptionOwner.badge.${availability}`)}
        </span>
        <span className="reception-card__meta">
          {t('receptionOwner.device', { name: record.deviceLabel })}
        </span>
      </div>
      <p className="reception-card__status" role="status">
        {t(`receptionOwner.${availability}`)}
      </p>
      {record.progress.pending || record.progress.retry ? (
        <p className="reception-card__notice">{t('receptionOwner.pending')}</p>
      ) : null}
      {record.status === 'idle' ? null : (
        // Taking over asks the responsible computer to stop; while it drains the request stays visible.
        <div className="reception-card__section">
          <p className="reception-card__hint">{t('receptionOwner.manualHint')}</p>
          <Button
            size="compact"
            tone="ghost"
            disabled={transfer.isPending || record.status !== 'active'}
            onClick={() => {
              transfer.mutate({ nextDeviceId: null });
            }}
          >
            {t('receptionOwner.manual')}
          </Button>
        </div>
      )}
      <details className="ar-disclosure">
        <summary>{t('receptionOwner.move')}</summary>
        <div className="ar-disclosure__body">
          <div className="reception-field-row">
            <label>
              {t('receptionOwner.choose')}
              <select
                value={target}
                onChange={(event) => {
                  setTarget(event.target.value);
                }}
              >
                <option value="">{t('receptionOwner.choose')}</option>
                {options.map((device) => (
                  <option key={device.deviceId} value={device.deviceId}>
                    {device.label}
                  </option>
                ))}
              </select>
            </label>
            <Button
              size="compact"
              disabled={transfer.isPending || !selected}
              onClick={() => {
                if (selected) transfer.mutate({ nextDeviceId: selected.deviceId });
              }}
            >
              {t('receptionOwner.transfer')}
            </Button>
          </div>
          <p className="reception-card__hint">{t('receptionOwner.transferHint')}</p>
          <Button
            size="compact"
            tone="ghost"
            onClick={() => {
              void copy();
            }}
          >
            {t(copied ? 'receptionOwner.copied' : 'receptionOwner.copy')}
          </Button>
          {availability === 'unreachable' || availability === 'draining' ? (
            <details className="ar-disclosure ar-disclosure--nested">
              <summary>{t('receptionOwner.recover')}</summary>
              <div className="ar-disclosure__body">
                <p className="reception-card__hint">{t('receptionOwner.recoverHint')}</p>
                <Button
                  tone="alert"
                  size="compact"
                  disabled={transfer.isPending || !selected}
                  onClick={() => {
                    if (selected)
                      transfer.mutate({ nextDeviceId: selected.deviceId, recover: true });
                  }}
                >
                  {t('receptionOwner.revoke')}
                </Button>
              </div>
            </details>
          ) : null}
        </div>
      </details>
      {cleanupPending ? <p role="status">{t('receptionOwner.cleanupPending')}</p> : null}
      {copyFailed || transfer.isError || (transfer.data?.ok === false && !cleanupPending) ? (
        <p role="alert">{t('receptionOwner.failed')}</p>
      ) : null}
    </article>
  );
}
