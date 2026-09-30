import { Banner, Button, Details, Dialog } from '@agent-room/ui-system';
import { Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import {
  agentActivity,
  instanceActivity,
  isNetworkInstance,
  liveInstances,
} from '@/features/workspace/domain/agent-card';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import { activityText } from '@/features/workspace/ui/agent-card-list';

export type AgentDeletionControl = {
  readonly canDelete: (agent: FleetAgent) => boolean;
  readonly pendingAgentId: string | null;
  readonly failure: { readonly agentId: string; readonly code: string } | null;
  readonly recentlyAuthenticated: boolean;
  readonly onDelete: (agent: FleetAgent) => void;
  readonly onReauthenticate: () => void;
};

/**
 * 一个 Agent 的详情：它在哪儿运行、每处是否在线，可以删除。ID 这类排查用的信息收在“详情”里。
 */
export function AgentDetailsDialog({
  agent,
  deletion,
  now,
  onClose,
}: {
  readonly agent: FleetAgent;
  readonly deletion?: AgentDeletionControl | undefined;
  readonly now: number;
  readonly onClose: () => void;
}) {
  const { i18n, t } = useTranslation();
  const language = i18n.resolvedLanguage;
  const connections = liveInstances(agent);
  return (
    <Dialog
      className="agent-details"
      closeLabel={t('agentInvite.close')}
      description={activityText(t, agentActivity(agent), now, language)}
      onClose={onClose}
      title={agent.agent.displayName}
    >
      <div className="agent-details__intro">
        <span aria-hidden="true" className="agent-details__portrait">
          <AgentPortrait id={agent.agent.agentId} />
        </span>
        {agent.agent.description === '' ? null : <p>{agent.agent.description}</p>}
      </div>
      <section aria-labelledby="agent-details-where" className="agent-details__where">
        <h3 id="agent-details-where">{t('workspace.details.where')}</h3>
        {connections.length === 0 ? (
          <p className="agent-details__muted">{t('workspace.details.nowhere')}</p>
        ) : (
          <ul>
            {connections.map((instance) => (
              <li data-status={instance.status} key={instance.agentInstanceId}>
                <span aria-hidden="true" className="agent-card__dot" />
                <strong>
                  {isNetworkInstance(instance)
                    ? t('workspace.place.network')
                    : instance.device.label}
                </strong>
                {instance.currentDevice ? (
                  <span className="agent-details__tag">{t('workspace.details.thisComputer')}</span>
                ) : null}
                <span className="agent-details__muted">
                  {activityText(t, instanceActivity(instance), now, language)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
      {deletion?.canDelete(agent) ? (
        <AgentDeletion agent={agent} deletion={deletion} key={agent.agent.agentId} />
      ) : null}
      <Details summary={t('workspace.details.more')}>
        <dl className="agent-details__ids">
          <div>
            <dt>{t('workspace.details.agentId')}</dt>
            <dd>
              <code>{agent.agent.agentId}</code>
            </dd>
          </div>
          <div>
            <dt>{t('workspace.details.matrixId')}</dt>
            <dd>
              <code>{agent.agent.matrixUserId}</code>
            </dd>
          </div>
          <div>
            <dt>{t('workspace.details.visibility')}</dt>
            <dd>{agent.agent.visibility}</dd>
          </div>
        </dl>
      </Details>
    </Dialog>
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
      <div className="agent-details__delete">
        <Button
          icon={<Trash2 aria-hidden="true" />}
          onClick={() => {
            setConfirming(true);
          }}
          size="compact"
          tone="ghost"
        >
          {t('workspace.delete')}
        </Button>
      </div>
    );
  }
  return (
    <section aria-label={t('workspace.delete')} className="agent-details__delete">
      <Banner
        icon={null}
        role={null}
        title={t('workspace.delete.title', { name: agent.agent.displayName })}
        tone="danger"
      >
        <p>{t('workspace.delete.body')}</p>
        {needsSignIn ? <p>{t('workspace.delete.signIn')}</p> : null}
      </Banner>
      <div className="agent-details__delete-actions">
        {needsSignIn ? (
          <Button onClick={deletion.onReauthenticate} size="compact" tone="primary">
            {t('workspace.delete.signInAgain')}
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
            {t(pending ? 'workspace.delete.pending' : 'workspace.delete.confirm')}
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
          {t('workspace.delete.cancel')}
        </Button>
      </div>
      {failure !== null && failure !== REAUTHENTICATION_REQUIRED ? (
        <p role="alert">
          {failure === 'agent.shared_ownership'
            ? t('workspace.delete.shared')
            : t('workspace.delete.failed', { code: failure })}
        </p>
      ) : null}
    </section>
  );
}
