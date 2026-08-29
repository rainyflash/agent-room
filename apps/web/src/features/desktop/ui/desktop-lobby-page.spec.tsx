// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { AppServicesProvider, type DesktopAppServices } from '@/app/app-services';
import type {
  DesktopLobbySnapshot,
  DesktopRuntimeGateway,
  DesktopRuntimeSnapshot,
} from '@/features/desktop/domain/desktop-runtime';
import { DesktopLobbyPage } from '@/features/desktop/ui/desktop-lobby-page';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

vi.mock('@tanstack/react-router', async (loadOriginal) => {
  const original = await loadOriginal<typeof import('@tanstack/react-router')>();
  return { ...original, useNavigate: () => vi.fn() };
});
vi.mock('@/features/lobby/ui/use-list-mode-requirement', () => ({
  useListModeRequirement: () => 'compact',
}));

const agentId = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const instanceId = '0198b601-77a4-7bb8-83eb-a8fe68c97e44';
const catalogId = '0198b601-77a2-7f41-b4f4-940f291951b8';
const roomId = '!public:matrix.test';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('桌面真实大厅', () => {
  it('Bridge 就绪后展示真实身份、状态和消息预览', async () => {
    const runtime = gateway();
    const services: DesktopAppServices = {
      config: {
        controlPlaneUrl: 'https://control.test',
        matrixHomeserverUrl: 'https://matrix.test',
        registrationMode: 'open-email',
        windowsDownloadUrl: null,
      },
      desktop: runtime,
      runtimeMode: 'desktop',
    };

    render(
      <I18nextProvider i18n={i18n}>
        <AppServicesProvider services={services}>
          <DesktopRuntimeProvider gateway={runtime}>
            <DesktopLobbyPage />
          </DesktopRuntimeProvider>
        </AppServicesProvider>
      </I18nextProvider>,
    );

    expect((await screen.findAllByText('Build Agent')).length).toBeGreaterThan(0);
    expect(screen.getByText('Working on release')).toBeVisible();
    expect(screen.getByText('Recent previews')).toBeVisible();
    expect(screen.queryByText('Bring your first Agent online.')).not.toBeInTheDocument();
  });
});

function gateway(): DesktopRuntimeGateway {
  const runtime = runtimeSnapshot();
  return {
    bootstrapDefaultAgent: async () =>
      ok({ agentId, lobbyLanguage: 'en', publicLobbyCatalogId: catalogId }),
    checkUpdate: async () =>
      ok({
        available: false,
        channel: 'testing',
        currentVersion: '0.1.0-alpha.5',
        rollback: false,
        sequence: 5,
        targetVersion: '0.1.0-alpha.5',
      }),
    configureAgentRuntime: async (target) => ok(target),
    installUpdate: async () => ok(undefined),
    isAvailable: () => true,
    openAuthorization: async () => ok(undefined),
    readLobby: async () => ok(lobbySnapshot()),
    retryBridge: async () => ok(runtime.bridge),
    setAutostart: async (enabled) => ok(enabled),
    snapshot: async () => ok(runtime),
    subscribe: async () => ok(() => undefined),
  };
}

function runtimeSnapshot(): DesktopRuntimeSnapshot {
  return {
    agentTarget: {
      agentId,
      lobbyLanguage: 'en',
      publicLobbyCatalogId: catalogId,
    },
    autostartEnabled: false,
    bridge: {
      authorization: null,
      lifecycle: {
        automaticRestartCount: 0,
        changedAtUnixMs: 1,
        diagnosticCode: null,
        lastExitCode: null,
        lastFailureCode: null,
        nextRetryAtUnixMs: null,
        ownership: 'managed',
        phase: 'ready',
      },
      session: { agentId, instanceId, matrixRoomId: roomId },
    },
    deepLink: null,
    platform: 'windows',
    updatesConfigured: true,
  };
}

function lobbySnapshot(): DesktopLobbySnapshot {
  const agent = {
    agentId,
    avatarUrl: null,
    displayName: 'Build Agent',
    matrixUserId: '@agent:matrix.test',
  } as const;
  return {
    agents: [
      {
        agent,
        instanceId,
        leaseExpiresAtUnixMs: 2_000,
        observedAtUnixMs: 1_000,
        roomId,
        status: 'working',
      },
    ],
    identity: {
      agent,
      connectionState: 'ready',
      grantedCapabilities: [],
      instanceId,
      matrixDeviceId: 'DEVICE',
      roomId,
    },
    messages: [
      {
        actor: { agent, instanceId, provenance: 'human_confirmed_agent' },
        content: {
          contentId: '0198b601-77a7-7bb8-83eb-a8fe68c97e44',
          digestSha256: '0'.repeat(64),
          mediaType: 'text/plain',
          sizeBytes: 12,
        },
        createdAtUnixMs: 1_100,
        eventId: '$message:matrix.test',
        language: 'en',
        messageId: '0198b601-77a8-7bb8-83eb-a8fe68c97e44',
        riskFlags: [],
        roomId,
        sensitivity: 'normal',
        summary: 'Release pipeline is healthy.',
        title: 'Working on release',
      },
    ],
    nextCursor: null,
    observedAtUnixMs: 1_200,
  };
}
