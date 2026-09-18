import { Button } from '@agent-room/ui-system';
import { Cpu, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import { formatWorkspaceTime } from '@/features/workspace/ui/workspace-format';

export type AgentDeletionControl = {
  readonly canDelete: (agent: FleetAgent) => boolean;
  readonly pendingAgentId: string | null;
  readonly failure: { readonly agentId: string; readonly code: string } | null;
  readonly recentlyAuthenticated: boolean;
  readonly onDelete: (agent: FleetAgent) => void;
  readonly onReauthenticate: () => void;
};

export function AgentInspector({
  agent,
  deletion,
}: {
  readonly agent: FleetAgent | null;
  readonly deletion?: AgentDeletionControl | undefined;
}) {
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
        <AgentPortrait id={agent.agent.agentId} />
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
      {deletion?.canDelete(agent) ? (
        <AgentDeletion agent={agent} deletion={deletion} key={agent.agent.agentId} />
      ) : null}
    </aside>
  );
}

const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

function AgentDeletion({
  agent,
  deletion,
}: {
  readonly agent: FleetAgent;
  readonly deletion: AgentDeletionControl;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  const pending = deletion.pendingAgentId === agent.agent.agentId;
  const failure = deletion.failure?.agentId === agent.agent.agentId ? deletion.failure.code : null;
  const needsSignIn = !deletion.recentlyAuthenticated || failure === REAUTHENTICATION_REQUIRED;
  if (!confirming) {
    return (
      <section className="workspace-agent-inspector__delete">
        <Button
          icon={<Trash2 aria-hidden="true" />}
          onClick={() => {
            setConfirming(true);
          }}
          size="compact"
          tone="ghost"
        >
          {t('workspace.inspector.delete')}
        </Button>
      </section>
    );
  }
  return (
    <section
      aria-label={t('workspace.inspector.delete')}
      className="workspace-agent-inspector__delete"
      data-confirming="true"
    >
      <strong>{t('workspace.inspector.deleteTitle', { name: agent.agent.displayName })}</strong>
      <p>{t('workspace.inspector.deleteBody')}</p>
      {needsSignIn ? <p>{t('workspace.inspector.deleteSignIn')}</p> : null}
      <div className="workspace-agent-inspector__delete-actions">
        {needsSignIn ? (
          <Button onClick={deletion.onReauthenticate} size="compact" tone="primary">
            {t('workspace.inspector.signInAgain')}
          </Button>
        ) : (
          <Button
            disabled={pending}
            onClick={() => {
              deletion.onDelete(agent);
            }}
            size="compact"
            tone="alert"
          >
            {t(pending ? 'workspace.inspector.deleting' : 'workspace.inspector.deleteConfirm')}
          </Button>
        )}
        <Button
          disabled={pending}
          onClick={() => {
            setConfirming(false);
          }}
          size="compact"
          tone="quiet"
        >
          {t('workspace.inspector.deleteCancel')}
        </Button>
      </div>
      {failure !== null && failure !== REAUTHENTICATION_REQUIRED ? (
        <p role="alert">
          {failure === 'agent.shared_ownership'
            ? t('workspace.inspector.deleteShared')
            : t('workspace.inspector.deleteFailed', { code: failure })}
        </p>
      ) : null}
    </section>
  );
}
