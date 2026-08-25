import { useMachine } from '@xstate/react';
import { Button, StatusMark, type StatusTone } from '@agent-room/ui-system';
import {
  ArrowLeft,
  Bot,
  Clock3,
  FileLock2,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  Send,
  ShieldAlert,
  type LucideIcon,
} from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useMemo, useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

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
} from '@/features/handoffs/domain/handoff';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { BrowserUuidV7Factory, type UuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { TranslationKey } from '@/shared/i18n/resources';

const browserHandoffIds = new BrowserUuidV7Factory();
const expiryMinutes = [5, 15, 60] as const;

const statusTone: Readonly<Record<HandoffStatus, StatusTone>> = Object.freeze({
  approved: 'network',
  consumed: 'active',
  declined: 'idle',
  delivered: 'active',
  expired: 'idle',
  failed: 'alert',
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

  const submit = (event: FormEvent<HTMLFormElement>): void => {
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
          <label className="handoff-field">
            <span>{t('handoff.field.target')}</span>
            <select
              onChange={(event) => {
                setSelectedInstanceId(event.target.value);
              }}
              value={selectedTarget.instanceId}
            >
              {delivery.context.targets.map((target) => (
                <option key={target.instanceId} value={target.instanceId}>
                  {target.displayName} · {shortId(target.instanceId)}
                </option>
              ))}
            </select>
          </label>
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
              <dd>{selectedTarget.displayName}</dd>
            </div>
            <div>
              <dt>{t('handoff.review.scope')}</dt>
              <dd>
                {permissions.map((permission) => t(`handoff.permission.${permission}`)).join(' · ')}
              </dd>
            </div>
            <div>
              <dt>{t('handoff.review.purpose')}</dt>
              <dd>{t(`handoff.purpose.${purpose}`)}</dd>
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
      {delivery.matches('uncertain') ? (
        <HandoffBoundary
          detail={t('handoff.uncertain.detail')}
          icon={Clock3}
          label={t('handoff.uncertain.title')}
        >
          <Button
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => {
              send({ type: 'QUERY' });
            }}
            size="compact"
            tone="quiet"
          >
            {t('handoff.query')}
          </Button>
        </HandoffBoundary>
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
        />
      ) : null}
      {delivery.matches('resolved') && delivery.context.snapshot !== null ? (
        <HandoffStatusView language={i18n.resolvedLanguage} snapshot={delivery.context.snapshot} />
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
}: {
  readonly language: string | undefined;
  readonly onQuery?: () => void;
  readonly onRevoke?: () => void;
  readonly snapshot: HandoffSnapshot;
}) {
  const { t } = useTranslation();
  return (
    <section className="handoff-panel__status" role="status">
      <StatusMark
        label={t(`handoff.status.${snapshot.status}`)}
        tone={statusTone[snapshot.status]}
      />
      <div>
        <strong>{t(`handoff.status.${snapshot.status}`)}</strong>
        <p>{t(`handoff.statusDetail.${snapshot.status}`)}</p>
        <code>{snapshot.handoffId}</code>
        <span>
          {t('handoff.expiresAt', {
            time: new Intl.DateTimeFormat(language, {
              dateStyle: 'medium',
              timeStyle: 'short',
            }).format(snapshot.expiresAtUnixMs),
          })}
        </span>
      </div>
      {onQuery === undefined && onRevoke === undefined ? null : (
        <div className="handoff-panel__status-actions">
          {onQuery === undefined ? null : (
            <Button onClick={onQuery} size="compact" tone="quiet">
              {t('handoff.query')}
            </Button>
          )}
          {onRevoke === undefined ? null : (
            <Button onClick={onRevoke} size="compact" tone="ghost">
              {t('handoff.revoke')}
            </Button>
          )}
        </div>
      )}
    </section>
  );
}

function shortId(value: string): string {
  return `${value.slice(0, 8)}…${value.slice(-4)}`;
}

const handoffFailureMessageKey: Readonly<Record<HandoffFailureCode, TranslationKey>> = {
  'handoff.already_resolved': 'handoff.failure.already_resolved',
  'handoff.authorization_denied': 'handoff.failure.authorization_denied',
  'handoff.bridge_unavailable': 'handoff.failure.bridge_unavailable',
  'handoff.invalid_intent': 'handoff.failure.invalid_intent',
  'handoff.not_found': 'handoff.failure.not_found',
  'handoff.persistence_failed': 'handoff.failure.persistence_failed',
  'handoff.targets_unavailable': 'handoff.failure.targets_unavailable',
  'handoff.transport_rejected': 'handoff.failure.transport_rejected',
  'handoff.unexpected_failure': 'handoff.failure.unexpected_failure',
};

function handoffFailureKey(code: HandoffFailureCode | undefined): TranslationKey {
  return handoffFailureMessageKey[code ?? 'handoff.unexpected_failure'];
}
