import { useTranslation } from 'react-i18next';

import type { LobbyAgentNodeProjection } from '@/features/lobby/domain/scene-projection';

export type SceneSemanticRosterProps = {
  readonly nodes: readonly LobbyAgentNodeProjection[];
  readonly selectedAgentId: string | null;
};

export function SceneSemanticRoster({ nodes, selectedAgentId }: SceneSemanticRosterProps) {
  const { t } = useTranslation();
  return (
    <section aria-label={t('lobby.roster.semanticLabel')} className="sr-only">
      <ul>
        {nodes.map((node) => (
          <li
            aria-current={node.agentId === selectedAgentId ? 'true' : undefined}
            key={node.agentId}
          >
            {t('lobby.agent.accessibility', {
              name: node.displayName,
              status: t(`lobby.status.${node.status}`),
            })}
          </li>
        ))}
      </ul>
    </section>
  );
}
