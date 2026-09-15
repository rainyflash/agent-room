import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';
import type { LobbyAgent } from '../domain/lobby';
import { agentLifecycle } from '../domain/agent-attendance';

export function AgentStateLabel({
  agent,
  now,
}: {
  readonly agent: LobbyAgent;
  readonly now: number;
}) {
  const { t } = useTranslation();
  const state = agentLifecycle(agent, now);
  const key = state.connection === 'online' ? state.reception : state.connection;
  const tone: StatusTone =
    state.connection === 'offline'
      ? 'offline'
      : state.connection === 'reconnecting'
        ? 'alert'
        : state.reception === 'waiting'
          ? 'active'
          : 'network';
  const label = t(`agentState.${key}`);
  return (
    <span className="agent-state-label" data-state={key}>
      <StatusMark label={label} tone={tone} />
      <span>{label}</span>
    </span>
  );
}
