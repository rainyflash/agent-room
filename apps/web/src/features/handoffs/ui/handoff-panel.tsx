import { initials } from '@/shared/ui/display-name';

import { Button, StatusMark, type StatusTone } from '@agent-room/ui-system';
import { useMachine } from '@xstate/react';
import {
  ArrowLeft,
  Bot,
  CircleDot,
  FileLock2,
  Laptop,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  Send,
  ShieldAlert,
  Wifi,
  WifiOff,
  type LucideIcon,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useMemo, useState, type SubmitEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { groupHandoffTargets } from '@/features/handoffs/application/group-handoff-targets';
import { createHandoffDeliveryMachine } from '@/features/handoffs/application/handoff-delivery-machine';
import {
  handoffPurposes,
  type HandoffApprovalRequest,
  type HandoffFailureCode,
  type HandoffGateway,
  type HandoffPermission,
  type HandoffPurpose,
  type HandoffSnapshot,
  type HandoffStatus,
  type HandoffTarget,
} from '@/features/handoffs/domain/handoff';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { BrowserUuidV7Factory, type UuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { TranslationKey } from '@/shared/i18n/resources';

import './handoff-panel.css';

const browserHandoffIds = new BrowserUuidV7Factory();
const expiryMinutes = [5, 15, 60] as const;

const statusTone: Readonly<Record<HandoffStatus, StatusTone>> = Object.freeze({
  consumed: 'active',
  declined: 'idle',
  delivered: 'active',
  expired: 'idle',
  failed: 'alert',
  queued: 'network',
  revoked: 'idle',
});

type HandoffReadyMessage = RoomMessageSignal & {
  readonly content: NonNullable<RoomMessageSignal['content']>;
  readonly preview: NonNullable<RoomMessageSignal['preview']>;
};

export type HandoffPanelProps = {
  readonly gateway: HandoffGateway;
  readonly handoffIds?: UuidV7Factory;
  readonly message: HandoffReadyMessage;
  readonly now?: () => number;
  readonly onBack: () => void;
};

export function HandoffPanel({
  gateway,
  handoffIds = browserHandoffIds,
  message,
  now = Date.now,
  onBack,
}: HandoffPanelProps) {
  const { i18n, t } = useTranslation();
  const reduceMotion = useReducedMotion();
  const machine = useMemo(
    () => createHandoffDeliveryMachine({ gateway, now, roomId: message.roomId }),
    [gateway, message.roomId, now],
  );
  const [delivery, send] = useMachine(machine);
  const [selectedInstanceId, setSelectedInstanceId] = useState<string | null>(null);
  const [purpose, setPurpose] = useState<HandoffPurpose>('summarize');
  const [includeMetadata, setIncludeMetadata] = useState(true);
  const [lifetimeMinutes, setLifetimeMinutes] = useState<(typeof expiryMinutes)[number]>(15);
  const selectedTarget =
    delivery.context.targets.find((target) => target.instanceId === selectedInstanceId) ??
    delivery.context.targets[0] ??
    null;
  const primaryPermission: HandoffPermission = message.content.mediaType.startsWith('text/')
    ? 'read_text'
    : 'read_attachments';
  const permissions: readonly HandoffPermission[] = includeMetadata
    ? [primaryPermission, 'include_metadata']
    : [primaryPermission];

  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!delivery.matches('ready') || selectedTarget === null) {
      return;
    }
    const approvedAtUnixMs = now();
    const request: HandoffApprovalRequest = {
      expiresAtUnixMs: approvedAtUnixMs + lifetimeMinutes * 60_000,
      handoffId: handoffIds.next(),
      permissions,
      purpose,
      source: {
        actor: message.actor,
        content: message.content,
        matrixEventId: message.matrixEventId,
        messageId: message.messageId,
        riskFlags: message.preview.riskFlags,
        roomId: message.roomId,
      },
      target: selectedTarget,
    };
    send({ request, type: 'SUBMIT' });
  };

  return (
    <motion.section
      animate={{ opacity: 1, x: 0 }}
      aria-labelledby="handoff-panel-title"
      className="handoff-panel"
      initial={reduceMotion === true ? false : { opacity: 0, x: 16 }}
      transition={{ bounce: 0.08, damping: 27, stiffness: 260, type: 'spring' }}
    >
      <header className="handoff-panel__header">
        <button
          aria-label={t('handoff.back')}
          className="handoff-panel__back"
          onClick={onBack}
          type="button"
        >
          <ArrowLeft aria-hidden="true" />
        </button>
        <div>
          <p className="eyebrow">{t('handoff.eyebrow')}</p>
          <h3 id="handoff-panel-title">{t('handoff.title')}</h3>
        </div>
        <span className="handoff-panel__cloud-badge">
          <CircleDot aria-hidden="true" />
          {t('handoff.cloudBadge')}
        </span>
      </header>
      <div className="handoff-panel__source">
        <FileLock2 aria-hidden="true" />
        <div>
          <span>{t('handoff.source')}</span>
          <strong>{message.preview.title}</strong>
          <code>{message.content.digestSha256.slice(0, 16)}…</code>
        </div>
      </div>
      {delivery.matches('resolvingTargets') ? (
        <HandoffProgress
          detail={t('handoff.progress.targetsDetail')}
          label={t('handoff.progress.targets')}
        />
      ) : null}
      {delivery.matches('ready') && delivery.context.targets.length === 0 ? (
        <HandoffBoundary
          detail={t('handoff.noTargets.detail')}
          icon={Bot}
          label={t('handoff.noTargets.title')}
        />
      ) : null}
      {delivery.matches('ready') && selectedTarget !== null ? (
        <form className="handoff-panel__form" onSubmit={submit}>
          <TargetDirectory
            language={i18n.resolvedLanguage}
            onSelect={setSelectedInstanceId}
            selectedInstanceId={selectedTarget.instanceId}
            targets={delivery.context.targets}
          />
          <section className="handoff-panel__authorization">
            <header>
              <span>{t('handoff.authorization.eyebrow')}</span>
              <strong>{t('handoff.authorization.title')}</strong>
              <p>{t('handoff.authorization.detail')}</p>
            </header>
            <fieldset className="handoff-fieldset">
              <legend>{t('handoff.field.scope')}</legend>
              <label>
                <input checked disabled type="checkbox" />
                <span>{t(`handoff.permission.${primaryPermission}`)}</span>
              </label>
              <label>
                <input
                  checked={includeMetadata}
                  onChange={(event) => {
                    setIncludeMetadata(event.target.checked);
                  }}
                  type="checkbox"
                />
                <span>{t('handoff.permission.include_metadata')}</span>
              </label>
            </fieldset>
            <fieldset className="handoff-fieldset handoff-fieldset--grid">
              <legend>{t('handoff.field.purpose')}</legend>
              {handoffPurposes.map((option) => (
                <label key={option}>
                  <input
                    checked={purpose === option}
                    name="handoff-purpose"
                    onChange={() => {
                      setPurpose(option);
                    }}
                    type="radio"
                    value={option}
                  />
                  <span>{t(`handoff.purpose.${option}`)}</span>
                </label>
              ))}
            </fieldset>
            <fieldset className="handoff-fieldset handoff-fieldset--expiry">
              <legend>{t('handoff.field.expiry')}</legend>
              {expiryMinutes.map((minutes) => (
                <label key={minutes}>
                  <input
                    checked={lifetimeMinutes === minutes}
                    name="handoff-expiry"
                    onChange={() => {
                      setLifetimeMinutes(minutes);
                    }}
                    type="radio"
                    value={minutes}
                  />
                  <span>{t('handoff.expiry.minutes', { count: minutes })}</span>
                </label>
              ))}
            </fieldset>
            <div className="handoff-panel__warning" role="note">
              <ShieldAlert aria-hidden="true" />
              <p>{t('handoff.warning')}</p>
            </div>
            <dl className="handoff-panel__review">
              <div>
                <dt>{t('handoff.review.target')}</dt>
                <dd>
                  {selectedTarget.agentDisplayName} · {selectedTarget.device.label}
                </dd>
              </div>
              <div>
                <dt>{t('handoff.review.delivery')}</dt>
                <dd>
                  {t(
                    selectedTarget.online
                      ? 'handoff.target.deliveryNow'
                      : 'handoff.target.deliveryQueued',
                  )}
                </dd>
              </div>
              <div>
                <dt>{t('handoff.review.scope')}</dt>
                <dd>
                  {permissions
                    .map((permission) => t(`handoff.permission.${permission}`))
                    .join(' · ')}
                </dd>
              </div>
              <div>
                <dt>{t('handoff.review.expiry')}</dt>
                <dd>{t('handoff.expiry.minutes', { count: lifetimeMinutes })}</dd>
              </div>
            </dl>
            <footer className="handoff-panel__footer">
              <span>{t('handoff.confirmDetail')}</span>
              <Button icon={<Send aria-hidden="true" />} tone="network" type="submit">
                {t('handoff.confirm')}
              </Button>
            </footer>
          </section>
        </form>
      ) : null}
      {delivery.matches('submitting') ? (
        <HandoffProgress
          detail={t('handoff.progress.submitDetail')}
          label={t('handoff.progress.submit')}
        />
      ) : null}
      {delivery.matches('reconciling') ? (
        <HandoffProgress
          detail={t('handoff.progress.queryDetail')}
          label={t('handoff.progress.query')}
        />
      ) : null}
      {delivery.matches('revoking') ? (
        <HandoffProgress
          detail={t('handoff.progress.revokeDetail')}
          label={t('handoff.progress.revoke')}
        />
      ) : null}
      {delivery.matches('active') && delivery.context.snapshot !== null ? (
        <HandoffStatusView
          language={i18n.resolvedLanguage}
          onQuery={() => {
            send({ type: 'QUERY' });
          }}
          onRevoke={() => {
            send({ type: 'REVOKE' });
          }}
          snapshot={delivery.context.snapshot}
          target={selectedTarget}
        />
      ) : null}
      {delivery.matches('resolved') && delivery.context.snapshot !== null ? (
        <HandoffStatusView
          language={i18n.resolvedLanguage}
          snapshot={delivery.context.snapshot}
          target={selectedTarget}
        />
      ) : null}
      {delivery.matches('failed') ? (
        <HandoffBoundary
          detail={t(handoffFailureKey(delivery.context.failure?.code))}
          icon={ShieldAlert}
          label={t('handoff.failed')}
        >
          {delivery.can({ type: 'RETRY' }) ? (
            <Button
              icon={<RotateCcw aria-hidden="true" />}
              onClick={() => {
                send({ type: 'RETRY' });
              }}
              size="compact"
              tone="quiet"
            >
              {t('handoff.retry')}
            </Button>
          ) : null}
        </HandoffBoundary>
      ) : null}
    </motion.section>
  );
}

function TargetDirectory({
  language,
  onSelect,
  selectedInstanceId,
  targets,
}: {
  readonly language: string | undefined;
  readonly onSelect: (instanceId: string) => void;
  readonly selectedInstanceId: string;
  readonly targets: readonly HandoffTarget[];
}) {
  const { t } = useTranslation();
  const groups = useMemo(() => groupHandoffTargets(targets), [targets]);
  return (
    <fieldset className="handoff-targets">
      <legend>{t('handoff.field.target')}</legend>
      <header className="handoff-targets__header">
        <div>
          <strong>{t('handoff.targets.title')}</strong>
          <span>{t('handoff.targets.agentCount', { count: groups.length })}</span>
        </div>
        <p>{t('handoff.targets.detail')}</p>
      </header>
      <div className="handoff-targets__list">
        {groups.map((agent) => (
          <section className="handoff-agent-group" key={agent.agentId}>
            <header>
              <span aria-hidden="true" className="handoff-agent-group__avatar">
                {initials(agent.agentDisplayName)}
              </span>
              <div>
                <strong>{agent.agentDisplayName}</strong>
                <code>{shortId(agent.agentId)}</code>
              </div>
            </header>
            {agent.devices.map(({ device, targets: deviceTargets }) => (
              <section className="handoff-device-group" key={device.deviceId}>
                <header>
                  <Laptop aria-hidden="true" />
                  <strong>{device.label}</strong>
                  <span>{t(`security.access.platform.${device.platform}`)}</span>
                </header>
                {deviceTargets.map((target) => (
                  <label
                    className="handoff-target"
                    data-online={target.online ? 'true' : 'false'}
                    key={target.instanceId}
                  >
                    <input
                      checked={selectedInstanceId === target.instanceId}
                      name="handoff-target"
                      onChange={() => {
                        onSelect(target.instanceId);
                      }}
                      type="radio"
                      value={target.instanceId}
                    />
                    <span className="handoff-target__availability">
                      {target.online ? <Wifi aria-hidden="true" /> : <WifiOff aria-hidden="true" />}
                    </span>
                    <span className="handoff-target__identity">
                      <strong>{target.adapterType}</strong>
                      <code>{shortId(target.instanceId)}</code>
                    </span>
                    <span className="handoff-target__state">
                      <strong>
                        {t(target.online ? 'handoff.target.online' : 'handoff.target.offline')}
                      </strong>
                      <small>
                        {target.lastSeenAtUnixMs === null
                          ? t('handoff.target.neverSeen')
                          : t('handoff.target.lastSeen', {
                              time: formatTargetTime(target.lastSeenAtUnixMs, language),
                            })}
                      </small>
                    </span>
                  </label>
                ))}
              </section>
            ))}
          </section>
        ))}
      </div>
    </fieldset>
  );
}

function HandoffProgress({ detail, label }: { readonly detail: string; readonly label: string }) {
  return (
    <section aria-live="polite" className="handoff-panel__progress" role="status">
      <LoaderCircle aria-hidden="true" />
      <div>
        <strong>{label}</strong>
        <p>{detail}</p>
      </div>
    </section>
  );
}

function HandoffBoundary({
  children,
  detail,
  icon: Icon,
  label,
}: {
  readonly children?: ReactNode;
  readonly detail: string;
  readonly icon: LucideIcon;
  readonly label: string;
}) {
  return (
    <section className="handoff-panel__boundary" role="status">
      <Icon aria-hidden="true" />
      <div>
        <strong>{label}</strong>
        <p>{detail}</p>
      </div>
      {children}
    </section>
  );
}

function HandoffStatusView({
  language,
  onQuery,
  onRevoke,
  snapshot,
  target,
}: {
  readonly language: string | undefined;
  readonly onQuery?: () => void;
  readonly onRevoke?: () => void;
  readonly snapshot: HandoffSnapshot;
  readonly target: HandoffTarget | null;
}) {
  const { t } = useTranslation();
  return (
    <section className="handoff-panel__status" role="status">
      <header>
        <StatusMark
          label={t(`handoff.status.${snapshot.status}`)}
          tone={statusTone[snapshot.status]}
        />
        <div>
          <p className="eyebrow">{t('handoff.status.eyebrow')}</p>
          <strong>{t(`handoff.status.${snapshot.status}`)}</strong>
          <p>{t(`handoff.statusDetail.${snapshot.status}`)}</p>
        </div>
      </header>
      <ol className="handoff-status-timeline">
        <TimelineStep
          active={snapshot.status === 'queued'}
          label={t('handoff.timeline.queued')}
          timestamp={snapshot.queuedAtUnixMs}
        />
        <TimelineStep
          active={snapshot.status === 'delivered'}
          label={t('handoff.timeline.delivered')}
          timestamp={snapshot.deliveredAtUnixMs}
        />
        <TimelineStep
          active={snapshot.status === 'consumed'}
          label={t('handoff.timeline.consumed')}
          timestamp={snapshot.consumedAtUnixMs}
        />
      </ol>
      <dl className="handoff-status-facts">
        <div>
          <dt>{t('handoff.review.target')}</dt>
          <dd>
            {target === null
              ? shortId(snapshot.targetInstanceId)
              : `${target.agentDisplayName} · ${target.device.label}`}
          </dd>
        </div>
        <div>
          <dt>{t('handoff.status.handoffId')}</dt>
          <dd>
            <code>{snapshot.handoffId}</code>
          </dd>
        </div>
        <div>
          <dt>{t('handoff.review.expiry')}</dt>
          <dd>
            {t('handoff.expiresAt', {
              time: new Intl.DateTimeFormat(language, {
                dateStyle: 'medium',
                timeStyle: 'short',
              }).format(snapshot.expiresAtUnixMs),
            })}
          </dd>
        </div>
        {snapshot.failureCode === null ? null : (
          <div>
            <dt>{t('handoff.status.failureCode')}</dt>
            <dd>
              <code>{snapshot.failureCode}</code>
            </dd>
          </div>
        )}
      </dl>
      {onQuery === undefined && onRevoke === undefined ? null : (
        <footer className="handoff-panel__status-actions">
          {onQuery === undefined ? null : (
            <Button
              icon={<RefreshCw aria-hidden="true" />}
              onClick={onQuery}
              size="compact"
              tone="quiet"
            >
              {t('handoff.query')}
            </Button>
          )}
          {onRevoke === undefined ? null : (
            <Button onClick={onRevoke} size="compact" tone="ghost">
              {t('handoff.revoke')}
            </Button>
          )}
        </footer>
      )}
    </section>
  );
}

function TimelineStep({
  active,
  label,
  timestamp,
}: {
  readonly active: boolean;
  readonly label: string;
  readonly timestamp: number | null;
}) {
  return (
    <li
      data-active={active ? 'true' : 'false'}
      data-complete={timestamp === null ? 'false' : 'true'}
    >
      <span aria-hidden="true" />
      <strong>{label}</strong>
    </li>
  );
}

function shortId(value: string): string {
  return `${value.slice(0, 8)}…${value.slice(-4)}`;
}

function formatTargetTime(value: number, language: string | undefined): string {
  return new Intl.DateTimeFormat(language, {
    dateStyle: 'short',
    timeStyle: 'short',
  }).format(value);
}

const handoffFailureMessageKey: Readonly<Record<HandoffFailureCode, TranslationKey>> = {
  'handoff.already_resolved': 'handoff.failure.already_resolved',
  'handoff.authorization_denied': 'handoff.failure.authorization_denied',
  'handoff.cloud_unavailable': 'handoff.failure.cloud_unavailable',
  'handoff.invalid_intent': 'handoff.failure.invalid_intent',
  'handoff.invalid_response': 'handoff.failure.invalid_response',
  'handoff.not_found': 'handoff.failure.not_found',
  'handoff.targets_unavailable': 'handoff.failure.targets_unavailable',
  'handoff.unexpected_failure': 'handoff.failure.unexpected_failure',
};

function handoffFailureKey(code: HandoffFailureCode | undefined): TranslationKey {
  return handoffFailureMessageKey[code ?? 'handoff.unexpected_failure'];
}
