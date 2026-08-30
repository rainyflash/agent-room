import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import { useDesktopRuntime } from '@/features/desktop/ui/use-desktop-runtime';
import {
  agentInstanceQueryKey,
  productDeviceQueryKey,
  useAgentInstances,
  useProductDevices,
} from '@/features/security/data/access-management-queries';
import type { MatrixSecuritySnapshot } from '@/features/security/domain/matrix-security';
import type { WebSession } from '@/features/session/domain/session';
import {
  ownedAgentQueryKey,
  useOwnedAgents,
} from '@/features/workspace/data/agent-directory-query';
import { projectAgentFleet } from '@/features/workspace/domain/agent-fleet';
import { AccountWorkspaceView } from '@/features/workspace/ui/account-workspace-view';
import { bridgeWorkspaceStatus } from '@/features/workspace/ui/connection-status-strip';

import './account-workspace-page.css';

export type AccountWorkspacePageProps = {
  readonly onSelectAgent: (agentId: string) => void;
  readonly principal: WebSession;
  readonly selectedAgentId: string | null;
};

export function AccountWorkspacePage({
  onSelectAgent,
  principal,
  selectedAgentId,
}: AccountWorkspacePageProps) {
  const services = useAppServices();
  const queryClient = useQueryClient();
  const agents = useOwnedAgents(services.agentDirectory);
  const devices = useProductDevices(services.accessManagement);
  const instances = useAgentInstances(services.accessManagement);
  const matrixSecurity = useQuery({
    networkMode: 'always',
    queryFn: async () => await services.security.inspect(),
    queryKey: ['matrix', 'security', 'workspace-current-device'] as const,
    retry: false,
    staleTime: 5_000,
  });
  const localRuntime = useDesktopRuntime(services.desktop);
  const fleet = useMemo(
    () =>
      projectAgentFleet({
        agents: resultValue(agents.data),
        currentMatrixDeviceId: currentMatrixDeviceId(matrixSecurity.data),
        devices: resultValue(devices.data),
        instances: resultValue(instances.data),
      }),
    [agents.data, devices.data, instances.data, matrixSecurity.data],
  );
  const failureCode =
    resultFailureCode(agents.data) ??
    resultFailureCode(devices.data) ??
    resultFailureCode(instances.data);

  const refresh = async (): Promise<void> => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ownedAgentQueryKey }),
      queryClient.invalidateQueries({ queryKey: productDeviceQueryKey }),
      queryClient.invalidateQueries({ queryKey: agentInstanceQueryKey }),
      matrixSecurity.refetch(),
    ]);
  };

  return (
    <AccountWorkspaceView
      bridgeStatus={bridgeWorkspaceStatus(
        localRuntime.available,
        localRuntime.snapshot?.bridge.lifecycle.phase,
      )}
      failureCode={failureCode}
      fleet={fleet}
      loading={agents.isPending || devices.isPending || instances.isPending}
      onRefresh={() => void refresh()}
      onSelectAgent={onSelectAgent}
      principalDisplayName={principal.displayName}
      selectedAgentId={selectedAgentId}
    />
  );
}

function resultValue<T>(
  result: { readonly ok: true; readonly value: readonly T[] } | { readonly ok: false } | undefined,
): readonly T[] {
  return result?.ok === true ? result.value : [];
}

function resultFailureCode(
  result:
    | { readonly error: { readonly code: string }; readonly ok: false }
    | { readonly ok: true }
    | undefined,
): string | null {
  return result?.ok === false ? result.error.code : null;
}

function currentMatrixDeviceId(
  result:
    | { readonly ok: true; readonly value: MatrixSecuritySnapshot }
    | { readonly ok: false }
    | undefined,
): string | null {
  return result?.ok === true ? result.value.currentDeviceId : null;
}
