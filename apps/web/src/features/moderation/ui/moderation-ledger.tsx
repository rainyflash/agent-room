import { Button, Details, StatusMark } from '@agent-room/ui-system';
import { FileWarning, RotateCcw, ScrollText, ShieldAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  moderationActionDisplayStatus,
  type ModerationAction,
  type ModerationAuditEvent,
  type ModerationCase,
} from '@/features/moderation/domain/moderation';

export function ModerationCaseLedger({ cases }: { readonly cases: readonly ModerationCase[] }) {
  const { i18n, t } = useTranslation();
  const formatter = dateFormatter(i18n.resolvedLanguage);
  return (
    <section aria-labelledby="moderation-cases-title" className="moderation-ledger">
      <header>
        <FileWarning aria-hidden="true" />
        <h3 id="moderation-cases-title">{t('moderation.governance.cases')}</h3>
        <span>{cases.length}</span>
      </header>
      {cases.length === 0 ? (
        <p className="moderation-ledger__empty">{t('moderation.governance.casesEmpty')}</p>
      ) : (
        <ol>
          {cases.map((moderationCase) => (
            <li key={moderationCase.caseId}>
              <div className="moderation-ledger__row">
                <div>
                  <strong>{t(`moderation.reason.${moderationCase.reason}`)}</strong>
                  <span>{formatter.format(moderationCase.createdAtUnixMs)}</span>
                </div>
                <StatusMark label={moderationCase.state} tone="network" />
              </div>
              {moderationCase.description === '' ? null : (
                <p className="moderation-ledger__description">{moderationCase.description}</p>
              )}
              {moderationCase.evidence.reporterSubmittedExcerpt === null ? (
                <p className="moderation-ledger__privacy">
                  {t('moderation.governance.case.noExcerpt')}
                </p>
              ) : (
                <figure className="moderation-ledger__excerpt">
                  <figcaption>{t('moderation.governance.case.explicitExcerpt')}</figcaption>
                  <blockquote>{moderationCase.evidence.reporterSubmittedExcerpt}</blockquote>
                </figure>
              )}
              {moderationCase.evidence.endToEndEncrypted ? (
                <span className="moderation-ledger__encrypted">
                  <ShieldAlert aria-hidden="true" />
                  {t('moderation.governance.case.encrypted')}
                </span>
              ) : null}
              <LedgerDetails
                facts={[
                  [
                    t(`moderation.target.${moderationCase.targetKind}`),
                    moderationCase.targetReference,
                  ],
                  [t('moderation.governance.caseId'), moderationCase.caseId],
                ]}
              />
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

export function ModerationActionLedger({
  actions,
  onReverse,
  pendingActionId,
  recentlyAuthenticated,
}: {
  readonly actions: readonly ModerationAction[];
  readonly onReverse: (actionId: string) => void;
  readonly pendingActionId: string | null;
  readonly recentlyAuthenticated: boolean;
}) {
  const { i18n, t } = useTranslation();
  const formatter = dateFormatter(i18n.resolvedLanguage);
  return (
    <section aria-labelledby="moderation-actions-title" className="moderation-ledger">
      <header>
        <ShieldAlert aria-hidden="true" />
        <h3 id="moderation-actions-title">{t('moderation.governance.actions')}</h3>
        <span>{actions.length}</span>
      </header>
      {actions.length === 0 ? (
        <p className="moderation-ledger__empty">{t('moderation.governance.actionsEmpty')}</p>
      ) : (
        <ol>
          {actions.map((action) => (
            <li key={action.actionId}>
              <div className="moderation-ledger__row">
                <div>
                  <strong>{t(`moderation.kind.${action.kind}`)}</strong>
                  <span>{formatter.format(action.startsAtUnixMs)}</span>
                  <ActionExpiry action={action} formatter={formatter} />
                </div>
                <StatusMark
                  label={t(`moderation.status.${moderationActionDisplayStatus(action)}`)}
                  tone={action.status === 'applied' ? 'network' : 'offline'}
                />
              </div>
              <LedgerDetails
                facts={[
                  [t(`moderation.target.${action.targetKind}`), action.targetReference],
                  ...(action.failureCode === null
                    ? []
                    : ([[t('moderation.governance.failureCode'), action.failureCode]] as const)),
                ]}
              />
              {action.status === 'applied' ? (
                <Button
                  disabled={!recentlyAuthenticated || pendingActionId !== null}
                  icon={<RotateCcw aria-hidden="true" />}
                  onClick={() => {
                    onReverse(action.actionId);
                  }}
                  size="compact"
                  tone="quiet"
                >
                  {t(
                    pendingActionId === action.actionId
                      ? 'moderation.governance.action.reversing'
                      : 'moderation.governance.action.reverse',
                  )}
                </Button>
              ) : null}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

export function ModerationAuditLedger({
  events,
}: {
  readonly events: readonly ModerationAuditEvent[];
}) {
  const { i18n, t } = useTranslation();
  const formatter = dateFormatter(i18n.resolvedLanguage);
  return (
    <section aria-labelledby="moderation-audit-title" className="moderation-ledger">
      <header>
        <ScrollText aria-hidden="true" />
        <h3 id="moderation-audit-title">{t('moderation.governance.audit')}</h3>
        <span>{events.length}</span>
      </header>
      {events.length === 0 ? (
        <p className="moderation-ledger__empty">{t('moderation.governance.auditEmpty')}</p>
      ) : (
        <ol>
          {events.map((event) => (
            <li key={event.eventId}>
              <div className="moderation-ledger__row">
                <div>
                  <strong>{event.action}</strong>
                  <span>{formatter.format(event.occurredAtUnixMs)}</span>
                </div>
                <StatusMark
                  label={t(`moderation.outcome.${event.outcome}`)}
                  tone={event.outcome === 'allowed' ? 'network' : 'offline'}
                />
              </div>
              <LedgerDetails
                facts={[
                  [t('moderation.governance.action.target'), event.targetReference],
                  [t('moderation.governance.correlation'), event.correlationId],
                ]}
              />
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

/** 限时的动作：生效中的说什么时候自动解除，到期解除了的说是哪个时候到期的。 */
function ActionExpiry({
  action,
  formatter,
}: {
  readonly action: ModerationAction;
  readonly formatter: Intl.DateTimeFormat;
}) {
  const { t } = useTranslation();
  if (action.expiresAtUnixMs === null) {
    return null;
  }
  const time = formatter.format(action.expiresAtUnixMs);
  if (action.status === 'applied') {
    return <span>{t('moderation.governance.action.endsAt', { time })}</span>;
  }
  if (moderationActionDisplayStatus(action) === 'expired') {
    return <span>{t('moderation.governance.action.expiredAt', { time })}</span>;
  }
  return null;
}

/** 消息 ID、举报 ID、错误码这些排查信息：收在每一项下面的详情里。 */
function LedgerDetails({
  facts,
}: {
  readonly facts: readonly (readonly [label: string, value: string])[];
}) {
  const { t } = useTranslation();
  return (
    <Details className="moderation-ledger__details" summary={t('moderation.governance.details')}>
      <dl>
        {facts.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>
              <code>{value}</code>
            </dd>
          </div>
        ))}
      </dl>
    </Details>
  );
}

function dateFormatter(language: string | undefined): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' });
}
