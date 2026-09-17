import { Button } from '@agent-room/ui-system';
import { Bot, Clock3, Gauge, ShieldCheck } from 'lucide-react';
import { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  partitionAutomationGrants,
  type AutomationGrant,
} from '@/features/automation/domain/automation-grant';
import type { AgentInstance } from '@/features/security/domain/access-management';

export type AutomationGrantListProps = {
  readonly grants: readonly AutomationGrant[];
  readonly instances: readonly AgentInstance[];
  readonly onRevoke: (grantId: string) => void;
  readonly pendingGrantId: string | null;
};

export function AutomationGrantList({
  grants,
  instances,
  onRevoke,
  pendingGrantId,
}: AutomationGrantListProps) {
  const { i18n, t } = useTranslation();
  const formatter = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });
  // 撤销和过期的授权会一直累积。用户来这个面板是问「现在谁能替我发言」，
  // 所以只有有效授权留在正文，历史收进折叠区，仍然可查但不再占据视线。
  const { active, history } = partitionAutomationGrants(grants, Date.now());
  // 但刚撤销掉的那一份是例外：用户点了撤销就要看到结果，把它藏进折叠区等于没有回执。
  const [historyOpen, setHistoryOpen] = useState(false);
  const seenHistoryCount = useRef(history.length);
  if (seenHistoryCount.current !== history.length) {
    const grew = history.length > seenHistoryCount.current;
    seenHistoryCount.current = history.length;
    if (grew) {
      setHistoryOpen(true);
    }
  }
  const row = (revocable: boolean) => (grant: AutomationGrant) => (
    <GrantRow
      formatter={formatter}
      grant={grant}
      instances={instances}
      key={grant.grantId}
      onRevoke={onRevoke}
      pendingGrantId={pendingGrantId}
      revocable={revocable}
    />
  );
  return (
    <section aria-labelledby="automation-grant-list-title" className="automation-grants">
      <header className="automation-section-heading automation-grants__heading">
        <span className="automation-section-heading__icon">
          <ShieldCheck aria-hidden="true" />
        </span>
        <div>
          <h2 id="automation-grant-list-title">{t('automation.grants.title')}</h2>
          <p>{t('automation.grants.count', { count: active.length })}</p>
        </div>
      </header>
      {active.length === 0 ? (
        <div className="automation-boundary">
          <Bot aria-hidden="true" />
          <div>
            <strong>{t('automation.empty')}</strong>
            <p>{t('automation.emptyDetail')}</p>
          </div>
        </div>
      ) : (
        <ol className="automation-grant-list">{active.map(row(true))}</ol>
      )}
      {history.length === 0 ? null : (
        <details
          className="automation-grant-history"
          onToggle={(event) => {
            setHistoryOpen(event.currentTarget.open);
          }}
          open={historyOpen}
        >
          <summary>{t('automation.grants.history', { count: history.length })}</summary>
          <p className="automation-grant-history__detail">{t('automation.grants.historyDetail')}</p>
          <ol className="automation-grant-list">{history.map(row(false))}</ol>
        </details>
      )}
    </section>
  );
}

type GrantRowProps = {
  readonly formatter: Intl.DateTimeFormat;
  readonly grant: AutomationGrant;
  readonly instances: readonly AgentInstance[];
  readonly onRevoke: (grantId: string) => void;
  readonly pendingGrantId: string | null;
  readonly revocable: boolean;
};

function GrantRow({
  formatter,
  grant,
  instances,
  onRevoke,
  pendingGrantId,
  revocable,
}: GrantRowProps) {
  const { t } = useTranslation();
  const instance = instances.find(
    (candidate) => candidate.agentInstanceId === grant.agentInstanceId,
  );
  const agent = instance ?? instances.find((candidate) => candidate.agentId === grant.agentId);
  const pending = pendingGrantId === grant.grantId;
  return (
    <li data-status={grant.status}>
      <div className="automation-grant-list__identity">
        <span className="automation-grant-list__avatar" aria-hidden="true">
          <Bot />
        </span>
        <div>
          <strong>{agent?.agentDisplayName ?? grant.agentId}</strong>
          <small>
            {t(
              grant.agentInstanceId === null
                ? 'automation.grant.agentWide'
                : 'automation.grant.exactInstance',
            )}
            {' · '}
            {grant.messageKinds.map((kind) => t(`automation.kind.${kind}`)).join(', ')}
          </small>
        </div>
        <span className={`automation-grant-status is-${grant.status}`}>
          {t(`automation.status.${grant.status}`)}
        </span>
      </div>
      <div className="automation-grant-list__metrics">
        <span>
          <Gauge aria-hidden="true" />
          {t('automation.grant.rate', {
            limit: grant.maxMessagesPerMinute,
            used: grant.messagesInCurrentMinute,
          })}
        </span>
        <span>
          <Bot aria-hidden="true" />
          {grant.maxTotalMessages === null
            ? t('automation.grant.totalOpen', { used: grant.totalMessages })
            : t('automation.grant.total', {
                limit: grant.maxTotalMessages,
                used: grant.totalMessages,
              })}
        </span>
        <span>
          <Clock3 aria-hidden="true" />
          {grant.revokedAtUnixMs === null
            ? t('automation.grant.expires', { time: formatter.format(grant.expiresAtUnixMs) })
            : t('automation.grant.revoked', { time: formatter.format(grant.revokedAtUnixMs) })}
        </span>
      </div>
      {revocable ? (
        <Button
          disabled={pendingGrantId !== null}
          onClick={() => {
            onRevoke(grant.grantId);
          }}
          size="compact"
          tone="alert"
        >
          {t(pending ? 'automation.action.revoking' : 'automation.action.revoke')}
        </Button>
      ) : null}
    </li>
  );
}
