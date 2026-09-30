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
import { ConnectionWorkspace } from '@/features/session/ui/connection-workspace';
import { useSession } from '@/features/session/ui/session-provider';
import { authenticationCallbackFailure } from '@/features/session/domain/authentication-callback';
import { AuthenticationRecovery } from '@/features/session/ui/authentication-recovery';
import { EntryShell } from '@/shared/ui/entry-shell';

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
      reachable: report.ok,
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
      void navigate({ to: '/rooms' });
      return;
    }
    send(eventByAction[action]);
  };

  const callbackFailure = authenticationCallbackFailure(
    `${window.location.pathname}${window.location.search}`,
  );
  if (callbackFailure !== null) {
    return (
      <EntryShell>
        <AuthenticationRecovery
          gateway={controlPlane}
          principal={snapshot.context.principal}
          reason={callbackFailure}
        />
      </EntryShell>
    );
  }

  return (
    <EntryShell>
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('connection.liveRegion', {
          detail: t(view.detailKey),
          stage: t(view.titleKey),
        })}
      </p>
      <ConnectionWorkspace context={snapshot.context} onAction={handleAction} view={view}>
        <DependencyHealthStrip
          matrixConnected={snapshot.context.connection !== null}
          online={window.navigator.onLine}
          readiness={readiness}
          sessionState={state}
        />
      </ConnectionWorkspace>
    </EntryShell>
  );
}
