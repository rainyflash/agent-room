import { useTranslation } from 'react-i18next';

import type { LobbyAgentNodeProjection } from '@/features/lobby/domain/scene-projection';

export type SceneSemanticRosterProps = {
  readonly activeAgentId: string | null;
  readonly nodes: readonly LobbyAgentNodeProjection[];
  readonly optionId: (agentId: string) => string;
};

export function SceneSemanticRoster({ activeAgentId, nodes, optionId }: SceneSemanticRosterProps) {
  const { t } = useTranslation();
  return (
    <>
      {nodes.map((node) => (
        <div
          aria-selected={node.agentId === activeAgentId}
          className="sr-only"
          id={optionId(node.agentId)}
          key={node.agentId}
          role="option"
        >
          {t('lobby.agent.accessibility', {
            name: node.displayName,
            status: t(`lobby.status.${node.status}`),
          })}
        </div>
      ))}
    </>
  );
}

export type SceneSelectionAnnouncementProps = {
  readonly activeAgentId: string | null;
  readonly instructionsId: string;
  readonly nodes: readonly LobbyAgentNodeProjection[];
};

export function SceneSelectionAnnouncement({
  activeAgentId,
  instructionsId,
  nodes,
}: SceneSelectionAnnouncementProps) {
  const { t } = useTranslation();
  const activeAgent = nodes.find((node) => node.agentId === activeAgentId) ?? null;
  return (
    <div className="sr-only">
      <p id={instructionsId}>{t('lobby.scene.instructions')}</p>
      <p aria-atomic="true" aria-live="polite">
        {activeAgent === null
          ? t('lobby.scene.noActiveAgent')
          : t('lobby.scene.activeAgent', {
              name: activeAgent.displayName,
              status: t(`lobby.status.${activeAgent.status}`),
            })}
      </p>
    </div>
  );
}
