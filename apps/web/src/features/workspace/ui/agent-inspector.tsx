import { Cpu } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import { formatWorkspaceTime } from '@/features/workspace/ui/workspace-format';

export function AgentInspector({ agent }: { readonly agent: FleetAgent | null }) {
  const { i18n, t } = useTranslation();
  if (agent === null) {
    return (
      <aside className="workspace-agent-inspector workspace-agent-inspector--empty">
        <Cpu aria-hidden="true" />
        <p>{t('workspace.inspector.empty')}</p>
      </aside>
    );
  }
  return (
    <aside className="workspace-agent-inspector">
      <header>
        <p className="eyebrow">{t('workspace.inspector.identity')}</p>
        <h2>{agent.agent.displayName}</h2>
        <code>{agent.agent.agentId}</code>
      </header>
      <dl className="workspace-agent-inspector__facts">
        <div>
          <dt>{t('workspace.inspector.visibility')}</dt>
          <dd>{agent.agent.visibility}</dd>
        </div>
        <div>
          <dt>{t('workspace.inspector.lastSeen')}</dt>
          <dd>
            {agent.lastSeenAtUnixMs === null
              ? t('workspace.inspector.neverSeen')
              : formatWorkspaceTime(agent.lastSeenAtUnixMs, i18n.resolvedLanguage)}
          </dd>
        </div>
      </dl>
      {agent.instances.length === 0 ? (
        <p className="workspace-agent-inspector__empty">{t('workspace.inspector.noInstances')}</p>
      ) : (
        <ol>
          {agent.instances.map((instance) => (
            <li key={instance.agentInstanceId}>
              <div className="workspace-agent-inspector__instance-heading">
                <span
                  className={`workspace-agent-inspector__instance-mark is-${instance.status}`}
                />
                <div>
                  <strong>{instance.device.label}</strong>
                  <small>
                    {t(
                      instance.currentDevice
                        ? 'workspace.inspector.instance.current'
                        : 'workspace.inspector.instance.remote',
                    )}
                  </small>
                </div>
                <span>{t(`workspace.status.${instance.status}`)}</span>
              </div>
              <p>
                {t('workspace.inspector.instance.adapter', {
                  adapter: instance.adapterType,
                  version: instance.capabilityVersion,
                })}
              </p>
              <time
                dateTime={new Date(
                  instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs,
                ).toISOString()}
              >
                {t('workspace.inspector.instance.lastSeen', {
                  time: formatWorkspaceTime(
                    instance.lastSeenAtUnixMs ?? instance.createdAtUnixMs,
                    i18n.resolvedLanguage,
                  ),
                })}
              </time>
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
