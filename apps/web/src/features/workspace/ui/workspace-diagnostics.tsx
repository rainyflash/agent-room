import { StatusMark } from '@agent-room/ui-system';
import { Activity, ChevronDown, CircleAlert } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type {
  WorkspaceConnectionHealth,
  WorkspaceLayerId,
} from '@/features/workspace/domain/connection-health';
import {
  WORKSPACE_LAYER_ORDER,
  workspaceStatusTone,
} from '@/features/workspace/ui/connection-status-strip';
import { formatWorkspaceTime } from '@/features/workspace/ui/workspace-format';

export function WorkspaceDiagnostics({
  health,
  orphanCount,
}: {
  readonly health: WorkspaceConnectionHealth;
  readonly orphanCount: number;
}) {
  const { i18n, t } = useTranslation();
  const issueCount = WORKSPACE_LAYER_ORDER.filter(
    (layerId) => health[layerId].status !== 'online',
  ).length;
  const [expanded, setExpanded] = useState(issueCount > 0 || orphanCount > 0);

  return (
    <details
      className="workspace-diagnostics"
      onToggle={(event) => setExpanded(event.currentTarget.open)}
      open={expanded}
    >
      <summary>
        <span className="workspace-diagnostics__summary-icon">
          {issueCount > 0 || orphanCount > 0 ? (
            <CircleAlert aria-hidden="true" />
          ) : (
            <Activity aria-hidden="true" />
          )}
        </span>
        <span>
          <strong>{t('workspace.diagnostic.title')}</strong>
          <small>
            {issueCount > 0
              ? t('workspace.diagnostic.issueSummary', { count: issueCount })
              : t('workspace.diagnostic.healthySummary')}
          </small>
        </span>
        <ChevronDown aria-hidden="true" className="workspace-diagnostics__chevron" />
      </summary>

      <div className="workspace-diagnostics__grid">
        {WORKSPACE_LAYER_ORDER.map((layerId) => (
          <DiagnosticLayer
            health={health[layerId]}
            key={layerId}
            label={t(layerLabelKey(layerId))}
            language={i18n.resolvedLanguage}
          />
        ))}
      </div>

      {orphanCount === 0 ? null : (
        <p className="workspace-diagnostics__orphans" role="status">
          <CircleAlert aria-hidden="true" />
          {t('workspace.diagnostic.orphans', { count: orphanCount })}
        </p>
      )}
    </details>
  );
}

function DiagnosticLayer({
  health,
  label,
  language,
}: {
  readonly health: WorkspaceConnectionHealth[WorkspaceLayerId];
  readonly label: string;
  readonly language: string | undefined;
}) {
  const { t } = useTranslation();
  return (
    <article className="workspace-diagnostics__layer">
      <header>
        <strong>{label}</strong>
        <StatusMark
          label={t(`workspace.status.${health.status}`)}
          pulse={health.status === 'connecting'}
          tone={workspaceStatusTone(health.status)}
        />
      </header>
      <dl>
        <div>
          <dt>{t('workspace.diagnostic.observed')}</dt>
          <dd>
            {health.observedAtUnixMs === null ? (
              t('workspace.diagnostic.neverObserved')
            ) : (
              <time dateTime={new Date(health.observedAtUnixMs).toISOString()}>
                {formatWorkspaceTime(health.observedAtUnixMs, language)}
              </time>
            )}
          </dd>
        </div>
        <div>
          <dt>{t('workspace.diagnostic.code')}</dt>
          <dd>
            {health.failureCode === null ? (
              t('workspace.diagnostic.noFailure')
            ) : (
              <code>{health.failureCode}</code>
            )}
          </dd>
        </div>
      </dl>
    </article>
  );
}

function layerLabelKey(layerId: WorkspaceLayerId) {
  const keys = {
    agents: 'workspace.agents.runtime',
    bridge: 'workspace.local.bridge',
    controlPlane: 'workspace.cloud.control',
    matrix: 'workspace.cloud.matrix',
  } as const;
  return keys[layerId];
}
