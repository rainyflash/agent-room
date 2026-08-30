import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from '@tanstack/react-router';
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';
import { projectAgentFleet } from '@/features/workspace/domain/agent-fleet';
import { projectWorkspaceConnectionHealth } from '@/features/workspace/domain/connection-health';
import { AccountWorkspaceView } from '@/features/workspace/ui/account-workspace-view';
import '@/features/workspace/ui/account-workspace-page.css';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const primaryAgentId = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const currentDeviceId = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';
const remoteDeviceId = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';
const fleet = projectAgentFleet({
  agents: [
    agent(primaryAgentId, 'Release Conductor', 'Coordinates verified release work.'),
    agent('0198b601-77a1-7bb8-83eb-a8fe68c97e45', 'Research Scout', 'Maps open questions.'),
  ],
  currentMatrixDeviceId: 'WEB-CURRENT',
  devices: [
    device(currentDeviceId, 'Studio workstation', 'WEB-CURRENT', 'windows'),
    device(remoteDeviceId, 'Travel notebook', 'WEB-REMOTE', 'linux'),
  ],
  instances: [
    instance(primaryAgentId, currentDeviceId, 'Studio workstation', 'codex', 'online'),
    instance(primaryAgentId, remoteDeviceId, 'Travel notebook', 'claude', 'degraded'),
  ],
});
const connectionHealth = projectWorkspaceConnectionHealth({
  agents: { failureCode: null, fleet, loading: false },
  bridge: {
    available: false,
    changedAtUnixMs: null,
    failureCode: null,
    phase: undefined,
  },
  controlPlane: {
    failureCode: null,
    observedAtUnixMs: Date.now() - 2_000,
    pending: false,
    results: [{ ok: true }, { ok: true }, { ok: true }],
  },
  matrix: {
    failureCode: null,
    observedAtUnixMs: Date.now() - 1_000,
    pending: false,
    result: { ok: true },
  },
});

const rootRoute = createRootRoute({ component: () => <Outlet /> });
const workspaceRoute = createRoute({
  component: WorkspaceFixture,
  getParentRoute: () => rootRoute,
  path: '/workspace',
});
const settingsRoute = createRoute({
  component: () => null,
  getParentRoute: () => rootRoute,
  path: '/settings/$section',
});
const onboardingRoute = createRoute({
  component: () => null,
  getParentRoute: () => rootRoute,
  path: '/onboarding',
});
const router = createRouter({
  history: createMemoryHistory({ initialEntries: ['/workspace'] }),
  routeTree: rootRoute.addChildren([workspaceRoute, settingsRoute, onboardingRoute]),
});

function WorkspaceFixture() {
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(primaryAgentId);
  return (
    <AccountWorkspaceView
      connectionHealth={connectionHealth}
      failureCode={null}
      fleet={fleet}
      loading={false}
      onRefresh={() => undefined}
      onSelectAgent={setSelectedAgentId}
      principalDisplayName="Fixture operator"
      selectedAgentId={selectedAgentId}
    />
  );
}

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage, ['en']);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('账号工作区测试根节点不存在。');
  }
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      <RouterProvider router={router} />
    </I18nextProvider>,
  );
}

function agent(agentId: string, displayName: string, description: string): OwnedAgent {
  return {
    agentId,
    avatarContentId: null,
    description,
    displayName,
    matrixUserId: `@_agent_${agentId.slice(-4)}:matrix.test`,
    registeredAtUnixMs: Date.now() - 86_400_000,
    slug: displayName.toLowerCase().replaceAll(' ', '-'),
    visibility: 'private',
  };
}

function device(
  deviceId: string,
  label: string,
  matrixDeviceId: string,
  platform: ProductDevice['platform'],
): ProductDevice {
  return {
    createdAtUnixMs: Date.now() - 86_400_000,
    deviceId,
    label,
    lastSeenAtUnixMs: Date.now() - 15_000,
    matrixDeviceId,
    platform,
    revokedAtUnixMs: null,
    trustState: 'verified',
  };
}

function instance(
  agentId: string,
  deviceId: string,
  label: string,
  adapterType: string,
  status: AgentInstance['status'],
): AgentInstance {
  return {
    adapterType,
    agentAvatarContentId: null,
    agentDisplayName: 'Release Conductor',
    agentId,
    agentInstanceId: `${agentId.slice(0, -1)}${deviceId.slice(-1)}`,
    capabilityVersion: '1.0',
    createdAtUnixMs: Date.now() - 60_000,
    device: { deviceId, label, platform: 'windows', trustState: 'verified' },
    lastSeenAtUnixMs: Date.now() - 15_000,
    matrixDeviceId: `AR-${deviceId.slice(-4)}`,
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status,
  };
}

void bootstrapFixture();
