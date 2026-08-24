import { useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { useReadiness } from '@/features/health/data/readiness-query';
import { DependencyHealthStrip } from '@/features/health/ui/dependency-health-strip';
import { failure } from '@/features/session/adapters/control-plane-client';
import {
  connectionViewModel,
  sessionStateName,
  type ConnectionAction,
} from '@/features/session/ui/connection-model';
import { ConnectionRail } from '@/features/session/ui/connection-rail';
import { ConnectionWorkspace } from '@/features/session/ui/connection-workspace';
import { useSession } from '@/features/session/ui/session-provider';

const eventByAction = {
  login: { type: 'LOGIN' },
  logout: { type: 'LOGOUT' },
  retry: { type: 'RETRY' },
} as const;

export function ConnectionPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { controlPlane } = useAppServices();
  const { send, snapshot } = useSession();
  const readiness = useReadiness(controlPlane);
  const state = sessionStateName(snapshot.value);
  const view = useMemo(
    () => connectionViewModel(state, snapshot.context),
    [snapshot.context, state],
  );

  useEffect(() => {
    const report = readiness.data;
    if (report === undefined) {
      return;
    }
    if (report.ok && report.value.status === 'ready') {
      send({ type: 'CONTROL_HEALTHY' });
      return;
    }
    send({
      type: 'CONTROL_DEGRADED',
      failure: report.ok
        ? failure(
            'control-plane',
            'control_plane.readiness_degraded',
            false,
            true,
            report.value.correlationId,
          )
        : report.error,
    });
  }, [readiness.data, send]);

  const handleAction = (action: ConnectionAction): void => {
    if (action === 'enter') {
      void navigate({ to: '/lobby/$catalogId', params: { catalogId: 'public' } });
      return;
    }
    send(eventByAction[action]);
  };

  return (
    <div className="connection-shell">
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('connection.liveRegion', {
          detail: t(view.detailKey),
          stage: t(view.titleKey),
        })}
      </p>
      <ConnectionRail stages={view.stages} />
      <ConnectionWorkspace context={snapshot.context} onAction={handleAction} view={view}>
        <DependencyHealthStrip
          matrixConnected={snapshot.context.connection !== null}
          online={window.navigator.onLine}
          readiness={readiness}
          sessionState={state}
        />
      </ConnectionWorkspace>
    </div>
  );
}
