import { Button } from '@agent-room/ui-system';
import { Bot, Clock3, Gauge, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  isAutomationGrantActive,
  orderAutomationGrants,
  type AutomationGrant,
} from '@/features/automation/domain/automation-grant';
import type { AgentInstance } from '@/features/security/domain/access-management';

export type AutomationGrantListProps = {
  readonly grants: readonly AutomationGrant[];
  readonly instances: readonly AgentInstance[];
  readonly onReauthenticate: () => void;
  readonly onRevoke: (grantId: string) => void;
  readonly pendingGrantId: string | null;
  readonly recentlyAuthenticated: boolean;
};

export function AutomationGrantList({
  grants,
  instances,
  onReauthenticate,
  onRevoke,
  pendingGrantId,
  recentlyAuthenticated,
}: AutomationGrantListProps) {
  const { i18n, t } = useTranslation();
  const formatter = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    dateStyle: 'medium',
    timeStyle: 'short',
  });
  return (
    <section aria-labelledby="automation-grant-list-title" className="automation-grants">
      <header className="automation-section-heading automation-grants__heading">
        <span className="automation-section-heading__icon">
          <ShieldCheck aria-hidden="true" />
        </span>
        <div>
          <h2 id="automation-grant-list-title">{t('automation.grants.title')}</h2>
          <p>{t('automation.grants.count', { count: grants.length })}</p>
        </div>
      </header>
      {grants.length === 0 ? (
        <div className="automation-boundary">
          <Bot aria-hidden="true" />
          <div>
            <strong>{t('automation.empty')}</strong>
            <p>{t('automation.emptyDetail')}</p>
          </div>
        </div>
      ) : (
        <ol className="automation-grant-list">
          {orderAutomationGrants(grants).map((grant) => {
            const instance = instances.find(
              (candidate) => candidate.agentInstanceId === grant.agentInstanceId,
            );
            const agent =
              instance ?? instances.find((candidate) => candidate.agentId === grant.agentId);
            const active = isAutomationGrantActive(grant, Date.now());
            const pending = pendingGrantId === grant.grantId;
            return (
              <li data-status={grant.status} key={grant.grantId}>
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
                      ? t('automation.grant.expires', {
                          time: formatter.format(grant.expiresAtUnixMs),
                        })
                      : t('automation.grant.revoked', {
                          time: formatter.format(grant.revokedAtUnixMs),
                        })}
                  </span>
                </div>
                {active ? (
                  recentlyAuthenticated ? (
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
                  ) : (
                    <Button onClick={onReauthenticate} size="compact" tone="quiet">
                      {t('automation.action.reauthenticate')}
                    </Button>
                  )
                ) : null}
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
