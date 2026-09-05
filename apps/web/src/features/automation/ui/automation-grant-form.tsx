import { Button } from '@agent-room/ui-system';
import { Bot, ShieldCheck } from 'lucide-react';
import { useMemo, useState, type SubmitEvent } from 'react';
import { useTranslation } from 'react-i18next';

import {
  automationMessageKinds,
  createAutomationGrantInputSchema,
  type AutomationAudience,
  type AutomationMessageKind,
  type CreateAutomationGrantInput,
} from '@/features/automation/domain/automation-grant';
import type { AgentInstance } from '@/features/security/domain/access-management';

const lifetimeOptions = [
  { label: 'hour', seconds: 60 * 60 },
  { label: 'workday', seconds: 8 * 60 * 60 },
  { label: 'day', seconds: 24 * 60 * 60 },
  { label: 'week', seconds: 7 * 24 * 60 * 60 },
  { label: 'month', seconds: 30 * 24 * 60 * 60 },
] as const;

export type AutomationGrantFormProps = {
  readonly catalogId: string;
  readonly instances: readonly AgentInstance[];
  readonly onCreate: (input: CreateAutomationGrantInput) => void;
  readonly onReauthenticate: () => void;
  readonly pending: boolean;
  readonly recentlyAuthenticated: boolean;
  readonly roomName: string;
};

export function AutomationGrantForm({
  catalogId,
  instances,
  onCreate,
  onReauthenticate,
  pending,
  recentlyAuthenticated,
  roomName,
}: AutomationGrantFormProps) {
  const { t } = useTranslation();
  const [instanceId, setInstanceId] = useState(instances[0]?.agentInstanceId ?? '');
  const [scope, setScope] = useState<'agent' | 'exact'>('exact');
  const [messageKinds, setMessageKinds] = useState<readonly AutomationMessageKind[]>([
    'room_message',
  ]);
  const [audience, setAudience] = useState<AutomationAudience>('known_room_members');
  const [rate, setRate] = useState('6');
  const [total, setTotal] = useState('100');
  const [lifetimeSeconds, setLifetimeSeconds] = useState(8 * 60 * 60);
  const [requiresRiskScan, setRequiresRiskScan] = useState(true);
  const [impactAcknowledged, setImpactAcknowledged] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const instance = instances.find((candidate) => candidate.agentInstanceId === instanceId) ?? null;
  const kindSummary = messageKinds.map((kind) => t(`automation.kind.${kind}`)).join(', ');
  const lifetimeLabel =
    lifetimeOptions.find((option) => option.seconds === lifetimeSeconds)?.label ?? 'workday';
  const impact = useMemo(
    () => ({
      agent: instance?.agentDisplayName ?? '—',
      audience: t(`automation.audience.${audience}`),
      kinds: kindSummary || '—',
      lifetime: t(`automation.lifetime.${lifetimeLabel}`),
      rate,
      scope: t(`automation.scope.${scope}`),
      total:
        total.trim() === ''
          ? t('automation.impact.totalUnbounded')
          : t('automation.impact.totalBounded', { count: Number(total) }),
    }),
    [audience, instance?.agentDisplayName, kindSummary, lifetimeLabel, rate, scope, t, total],
  );

  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const candidate: unknown = {
      agentId: instance?.agentId,
      ...(scope === 'exact' ? { agentInstanceId: instance?.agentInstanceId } : {}),
      audience,
      impactAcknowledged,
      lifetimeSeconds,
      maxMessagesPerMinute: Number(rate),
      ...(total.trim() === '' ? {} : { maxTotalMessages: Number(total) }),
      messageKinds,
      requiresRiskScan,
      roomCatalogId: catalogId,
    };
    const parsed = createAutomationGrantInputSchema.safeParse(candidate);
    if (!parsed.success) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    onCreate(parsed.data);
  };

  const toggleKind = (kind: AutomationMessageKind): void => {
    setMessageKinds((current) =>
      current.includes(kind) ? current.filter((value) => value !== kind) : [...current, kind],
    );
  };

  return (
    <form className="automation-form" onSubmit={submit}>
      <header className="automation-section-heading">
        <span className="automation-section-heading__icon">
          <Bot aria-hidden="true" />
        </span>
        <div>
          <h2>{t('automation.create.title')}</h2>
          <p>{t('automation.create.detail')}</p>
        </div>
      </header>

      {instances.length === 0 ? (
        <div className="automation-boundary">
          <Bot aria-hidden="true" />
          <div>
            <strong>{t('automation.noInstances.title')}</strong>
            <p>{t('automation.noInstances.detail')}</p>
          </div>
        </div>
      ) : (
        <>
          <div className="automation-form__grid">
            <label className="automation-field automation-field--wide">
              <span>{t('automation.field.instance')}</span>
              <select
                disabled={pending}
                onChange={(event) => {
                  setInstanceId(event.target.value);
                }}
                value={instanceId}
              >
                {instances.map((candidate) => (
                  <option key={candidate.agentInstanceId} value={candidate.agentInstanceId}>
                    {candidate.agentDisplayName} · {candidate.device.label} ·{' '}
                    {candidate.adapterType}
                  </option>
                ))}
              </select>
            </label>

            <fieldset className="automation-field automation-field--wide">
              <legend>{t('automation.field.instanceScope')}</legend>
              <div className="automation-segmented">
                {(['exact', 'agent'] as const).map((value) => (
                  <label key={value}>
                    <input
                      checked={scope === value}
                      disabled={pending}
                      name="automation-scope"
                      onChange={() => {
                        setScope(value);
                      }}
                      type="radio"
                    />
                    <span>{t(`automation.scope.${value}`)}</span>
                  </label>
                ))}
              </div>
            </fieldset>

            <fieldset className="automation-field automation-field--wide">
              <legend>{t('automation.field.messageKinds')}</legend>
              <div className="automation-checks">
                {automationMessageKinds.map((kind) => (
                  <label key={kind}>
                    <input
                      checked={messageKinds.includes(kind)}
                      disabled={pending}
                      onChange={() => {
                        toggleKind(kind);
                      }}
                      type="checkbox"
                    />
                    <span>{t(`automation.kind.${kind}`)}</span>
                  </label>
                ))}
              </div>
            </fieldset>

            <label className="automation-field">
              <span>{t('automation.field.audience')}</span>
              <select
                disabled={pending}
                onChange={(event) => {
                  setAudience(event.target.value as AutomationAudience);
                }}
                value={audience}
              >
                <option value="known_room_members">
                  {t('automation.audience.known_room_members')}
                </option>
                <option value="any_room_member">{t('automation.audience.any_room_member')}</option>
              </select>
            </label>

            <label className="automation-field">
              <span>{t('automation.field.lifetime')}</span>
              <select
                disabled={pending}
                onChange={(event) => {
                  setLifetimeSeconds(Number(event.target.value));
                }}
                value={lifetimeSeconds}
              >
                {lifetimeOptions.map((option) => (
                  <option key={option.seconds} value={option.seconds}>
                    {t(`automation.lifetime.${option.label}`)}
                  </option>
                ))}
              </select>
            </label>

            <label className="automation-field">
              <span>{t('automation.field.rate')}</span>
              <input
                disabled={pending}
                inputMode="numeric"
                max={60}
                min={1}
                onChange={(event) => {
                  setRate(event.target.value);
                }}
                type="number"
                value={rate}
              />
            </label>

            <label className="automation-field">
              <span>{t('automation.field.total')}</span>
              <input
                aria-describedby="automation-total-hint"
                disabled={pending}
                inputMode="numeric"
                max={10_000}
                min={1}
                onChange={(event) => {
                  setTotal(event.target.value);
                }}
                type="number"
                value={total}
              />
              <small id="automation-total-hint">{t('automation.field.totalHint')}</small>
            </label>
          </div>

          <label className="automation-switch">
            <input
              checked={requiresRiskScan}
              disabled={pending}
              onChange={(event) => {
                setRequiresRiskScan(event.target.checked);
              }}
              type="checkbox"
            />
            <span aria-hidden="true" />
            <strong>{t('automation.field.riskScan')}</strong>
          </label>

          <section aria-labelledby="automation-impact-title" className="automation-impact">
            <ShieldCheck aria-hidden="true" />
            <div>
              <p className="eyebrow">{t('automation.impact.eyebrow')}</p>
              <h3 id="automation-impact-title">
                {t('automation.impact.title', { agent: impact.agent, room: roomName })}
              </h3>
              <p>{t('automation.impact.detail', impact)}</p>
              <strong>
                {t(
                  requiresRiskScan
                    ? 'automation.impact.riskRequired'
                    : 'automation.impact.riskSkipped',
                )}
              </strong>
            </div>
          </section>

          <label className="automation-acknowledgement">
            <input
              checked={impactAcknowledged}
              disabled={pending}
              onChange={(event) => {
                setImpactAcknowledged(event.target.checked);
              }}
              type="checkbox"
            />
            <span>{t('automation.impact.acknowledge')}</span>
          </label>

          {invalid ? (
            <p className="automation-inline-failure" role="alert">
              {t('automation.validation')}
            </p>
          ) : null}

          <div className="automation-form__action">
            {recentlyAuthenticated ? (
              <Button
                disabled={pending || !impactAcknowledged}
                size="default"
                tone="primary"
                type="submit"
              >
                {t(pending ? 'automation.action.creating' : 'automation.action.create')}
              </Button>
            ) : (
              <div className="automation-reauth">
                <div>
                  <strong>{t('automation.recentAuth.title')}</strong>
                  <p>{t('automation.recentAuth.detail')}</p>
                </div>
                <Button onClick={onReauthenticate} size="default" tone="primary" type="button">
                  {t('automation.action.reauthenticate')}
                </Button>
              </div>
            )}
          </div>
        </>
      )}
    </form>
  );
}
