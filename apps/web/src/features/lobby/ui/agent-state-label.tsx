import { useTranslation } from 'react-i18next';
import type { LobbyAgent } from '../domain/lobby';
import { agentStateKey } from '../domain/agent-attendance';

/** 一个 Agent 的状态，写成字。点在它的头像上（`LobbyAgentAvatar`），这里不再画。 */
export function AgentStateLabel({
  agent,
  now,
}: {
  readonly agent: LobbyAgent;
  readonly now: number;
}) {
  const { t } = useTranslation();
  const key = agentStateKey(agent, now);
  return (
    <span className="agent-state-label" data-state={key}>
      {t(`agentState.${key}`)}
    </span>
  );
}
