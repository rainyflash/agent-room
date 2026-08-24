import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import type { UseQueryResult } from '@tanstack/react-query';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import type { ReadinessReport } from '@/features/health/domain/readiness';
import type { SessionFailure } from '@/features/session/domain/session';
import type { SessionStateName } from '@/features/session/ui/connection-model';
import type { Result } from '@/shared/result';

export type DependencyHealthStripProps = {
  readonly matrixConnected: boolean;
  readonly online: boolean;
  readonly readiness: UseQueryResult<Result<ReadinessReport, SessionFailure>>;
  readonly sessionState: SessionStateName;
};

type HealthItem = {
  readonly detail: string;
  readonly label: string;
  readonly tone: StatusTone;
};

export function DependencyHealthStrip({
  matrixConnected,
  online,
  readiness,
  sessionState,
}: DependencyHealthStripProps) {
  const { i18n, t } = useTranslation();
  const items = healthItems({ matrixConnected, online, readiness, sessionState, t });
  const checkedAt = readiness.data?.ok === true ? readiness.data.value.checkedAtUnixMs : null;

  return (
    <section aria-label={t('connection.health.title')} className="health-strip">
      <div className="health-strip__heading">
        <span>{t('connection.health.title')}</span>
        <time dateTime={checkedAt === null ? undefined : new Date(checkedAt).toISOString()}>
          {checkedAt === null
            ? t('connection.health.pending')
            : t('connection.health.checkedAt', {
                time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
                  hour: '2-digit',
                  minute: '2-digit',
                  second: '2-digit',
                }).format(checkedAt),
              })}
        </time>
      </div>
      <div className="health-strip__items">
        {items.map((item) => (
          <article className="health-item" key={item.label}>
            <StatusMark label={item.detail} tone={item.tone} />
            <div>
              <span>{item.label}</span>
              <strong>{item.detail}</strong>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}

type HealthItemInput = Omit<DependencyHealthStripProps, 'readiness'> & {
  readonly readiness: UseQueryResult<Result<ReadinessReport, SessionFailure>>;
  readonly t: TFunction;
};

function healthItems({
  matrixConnected,
  online,
  readiness,
  sessionState,
  t,
}: HealthItemInput): readonly HealthItem[] {
  const control = controlHealth(readiness, t);
  const matrix: HealthItem = matrixConnected
    ? {
        detail: t('connection.health.ready'),
        label: t('connection.health.matrix'),
        tone: 'active',
      }
    : sessionState === 'degraded'
      ? {
          detail: t('connection.health.degraded'),
          label: t('connection.health.matrix'),
          tone: 'alert',
        }
      : sessionState === 'offline'
        ? {
            detail: t('connection.health.offline'),
            label: t('connection.health.matrix'),
            tone: 'offline',
          }
        : sessionState === 'restoring' ||
            sessionState === 'syncing' ||
            sessionState === 'reconnecting'
          ? {
              detail: t('connection.health.pending'),
              label: t('connection.health.matrix'),
              tone: 'network',
            }
          : {
              detail: t('connection.health.pending'),
              label: t('connection.health.matrix'),
              tone: 'network',
            };
  const network: HealthItem = online
    ? {
        detail: t('connection.health.ready'),
        label: t('connection.health.network'),
        tone: 'active',
      }
    : {
        detail: t('connection.health.offline'),
        label: t('connection.health.network'),
        tone: 'offline',
      };
  return [control, matrix, network];
}

function controlHealth(
  readiness: UseQueryResult<Result<ReadinessReport, SessionFailure>>,
  t: HealthItemInput['t'],
): HealthItem {
  const result = readiness.data;
  if (result === undefined) {
    return {
      detail: t('connection.health.pending'),
      label: t('connection.health.control'),
      tone: 'network',
    };
  }
  if (!result.ok || result.value.status === 'degraded') {
    return {
      detail: t('connection.health.degraded'),
      label: t('connection.health.control'),
      tone: 'alert',
    };
  }
  const latency = Math.max(0, ...result.value.dependencies.map(({ latencyMs }) => latencyMs));
  return {
    detail: t('connection.health.latency', { latency }),
    label: t('connection.health.control'),
    tone: 'active',
  };
}
