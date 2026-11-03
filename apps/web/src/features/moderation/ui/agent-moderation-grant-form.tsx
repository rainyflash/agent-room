import { Button } from '@agent-room/ui-system';
import { Bot, CircleAlert, LoaderCircle } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  grantRoomModerationInputSchema,
  MAX_MODERATION_RULES_CHARACTERS,
  roomModerationPowers,
  type GrantRoomModerationInput,
  type RoomModerationPower,
} from '@/features/moderation/domain/agent-moderation';

const lifetimeOptions = [
  { label: 'hour', seconds: 60 * 60 },
  { label: 'day', seconds: 24 * 60 * 60 },
  { label: 'week', seconds: 7 * 24 * 60 * 60 },
  { label: 'month', seconds: 30 * 24 * 60 * 60 },
] as const;

/** 能交给它管的 Agent：同一个 Agent 在几台电脑上只列一次。 */
export type GrantableAgent = { readonly agentId: string; readonly displayName: string };

export type AgentModerationGrantFormProps = {
  readonly agents: readonly GrantableAgent[];
  readonly onGrant: (input: GrantRoomModerationInput) => void;
  readonly onReauthenticate: () => void;
  readonly pending: boolean;
  readonly recentlyAuthenticated: boolean;
  readonly roomName: string;
};

/**
 * 交给 Agent 管：选自己的哪个 Agent、写规则、勾它能做的、多久。默认只勾禁言，再给同一个 Agent 一次盖掉上一份。
 */
export function AgentModerationGrantForm({
  agents,
  onGrant,
  onReauthenticate,
  pending,
  recentlyAuthenticated,
  roomName,
}: AgentModerationGrantFormProps) {
  const { t } = useTranslation();
  const [agentId, setAgentId] = useState(agents[0]?.agentId ?? '');
  const [rules, setRules] = useState('');
  const [powers, setPowers] = useState<readonly RoomModerationPower[]>(['mute']);
  const [lifetimeSeconds, setLifetimeSeconds] = useState<number>(lifetimeOptions[3].seconds);
  const [attempted, setAttempted] = useState(false);
  const agent = agents.find((candidate) => candidate.agentId === agentId) ?? agents[0];
  const input = grantRoomModerationInputSchema.safeParse({
    actions: roomModerationPowers.filter((power) => powers.includes(power)),
    agentId: agent?.agentId,
    lifetimeSeconds,
    rules,
  });

  if (agent === undefined) {
    return (
      <div className="moderation-boundary" role="note">
        <Bot aria-hidden="true" />
        <span>{t('moderation.agents.noAgents')}</span>
      </div>
    );
  }

  const toggle = (power: RoomModerationPower): void => {
    setPowers((current) =>
      current.includes(power) ? current.filter((value) => value !== power) : [...current, power],
    );
  };

  return (
    <form
      className="moderation-action-form moderation-agent-grant"
      onSubmit={(event) => {
        event.preventDefault();
        setAttempted(true);
        if (input.success && recentlyAuthenticated) onGrant(input.data);
      }}
    >
      <header>
        <Bot aria-hidden="true" />
        <div>
          <h3>{t('moderation.agents.title')}</h3>
          <p>{t('moderation.agents.detail', { room: roomName })}</p>
        </div>
      </header>
      <div className="moderation-action-form__grid">
        <label>
          <span>{t('moderation.agents.field.agent')}</span>
          <select
            disabled={pending}
            onChange={(event) => {
              setAgentId(event.currentTarget.value);
            }}
            value={agent.agentId}
          >
            {agents.map((candidate) => (
              <option key={candidate.agentId} value={candidate.agentId}>
                {candidate.displayName}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>{t('moderation.agents.field.lifetime')}</span>
          <select
            disabled={pending}
            onChange={(event) => {
              setLifetimeSeconds(Number(event.currentTarget.value));
            }}
            value={lifetimeSeconds}
          >
            {lifetimeOptions.map((option) => (
              <option key={option.seconds} value={option.seconds}>
                {t(`moderation.agents.lifetime.${option.label}`)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <label>
        <span>{t('moderation.agents.field.rules')}</span>
        <textarea
          disabled={pending}
          maxLength={MAX_MODERATION_RULES_CHARACTERS * 2}
          onChange={(event) => {
            setRules(event.currentTarget.value);
          }}
          placeholder={t('moderation.agents.field.rulesPlaceholder')}
          rows={4}
          value={rules}
        />
      </label>
      <fieldset className="moderation-agent-grant__powers">
        <legend>{t('moderation.agents.field.powers')}</legend>
        {roomModerationPowers.map((power) => (
          <label key={power}>
            <input
              checked={powers.includes(power)}
              disabled={pending}
              onChange={() => {
                toggle(power);
              }}
              type="checkbox"
            />
            <span>{t(`moderation.agents.power.${power}`)}</span>
          </label>
        ))}
      </fieldset>
      <p className="moderation-agent-grant__impact">
        {t('moderation.agents.impact', { agent: agent.displayName })}
      </p>
      {!recentlyAuthenticated ? (
        <div className="moderation-inline-failure" role="note">
          <CircleAlert aria-hidden="true" />
          <span>{t('moderation.governance.action.recentAuth')}</span>
          <Button onClick={onReauthenticate} size="compact" tone="quiet">
            {t('moderation.governance.action.reauthenticate')}
          </Button>
        </div>
      ) : null}
      {attempted && !input.success ? (
        <p className="moderation-form-validation" role="alert">
          {t('moderation.agents.invalid')}
        </p>
      ) : null}
      <Button
        disabled={pending || !recentlyAuthenticated}
        icon={pending ? <LoaderCircle aria-hidden="true" /> : <Bot aria-hidden="true" />}
        tone="primary"
        type="submit"
      >
        {t(pending ? 'moderation.agents.granting' : 'moderation.agents.grant')}
      </Button>
    </form>
  );
}
