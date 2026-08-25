import { Button, StatusMark } from '@agent-room/ui-system';
import { FileWarning, RotateCcw, ScrollText, ShieldAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type {
  ModerationAction,
  ModerationAuditEvent,
  ModerationCase,
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
                  <code>{moderationCase.caseId}</code>
                </div>
                <StatusMark label={moderationCase.state} tone="network" />
              </div>
              <dl className="moderation-ledger__facts">
                <div>
                  <dt>{t(`moderation.target.${moderationCase.targetKind}`)}</dt>
                  <dd>{moderationCase.targetReference}</dd>
                </div>
                <div>
                  <dt>{formatter.format(moderationCase.createdAtUnixMs)}</dt>
                  <dd>{moderationCase.description}</dd>
                </div>
              </dl>
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
                </div>
                <StatusMark
                  label={t(`moderation.status.${action.status}`)}
                  tone={action.status === 'applied' ? 'network' : 'offline'}
                />
              </div>
              <code className="moderation-ledger__target">{action.targetReference}</code>
              {action.failureCode === null ? null : (
                <p className="moderation-ledger__privacy">{action.failureCode}</p>
              )}
              {action.status === 'applied' ? (
                <Button
                  disabled={!recentlyAuthenticated || pendingActionId !== null}
                  icon={<RotateCcw aria-hidden="true" />}
                  onClick={() => onReverse(action.actionId)}
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
              <code className="moderation-ledger__target">{event.targetReference}</code>
              <p className="moderation-ledger__privacy">{event.correlationId}</p>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function dateFormatter(language: string | undefined): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' });
}
