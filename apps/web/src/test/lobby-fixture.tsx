import { createRoot } from 'react-dom/client';
import { useState } from 'react';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import type {
  LobbyAgent,
  LobbyAgentStatus,
  LobbyGateway,
  LobbyRoom,
} from '@/features/lobby/domain/lobby';
import { LobbyPage } from '@/features/lobby/ui/lobby-page';
import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { MessageGateway } from '@/features/messages/domain/message';
import type {
  MessagePublicationRequest,
  MessagePublisher,
  PublicationProgressStage,
} from '@/features/messages/domain/publication';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const room = testRoom(200);
const lobby: LobbyGateway = {
  read: () => ok(room),
  subscribe: () => noop,
};
const messages: MessageGateway = {
  read: () =>
    ok({
      messages: [],
      observedAtUnixMs: Date.now(),
      roomId: room.roomId,
    }),
  subscribe: () => noop,
};
const content: ContentGateway = {
  download: () => Promise.resolve(err({ code: 'content.download_rejected', retryable: false })),
  issueReadTicket: () =>
    Promise.resolve(err({ code: 'content.ticket_rejected', retryable: false })),
};
const contentVerifier: ContentVerifier = {
  verify: () => Promise.resolve(err({ code: 'content.invalid_response', retryable: false })),
};

class FixtureMessagePublisher implements MessagePublisher {
  publish(
    request: MessagePublicationRequest,
    onProgress: (stage: PublicationProgressStage) => void,
  ) {
    onProgress('submitting');
    return Promise.resolve(
      ok({
        kind: 'published' as const,
        matrixEventId: '$fixture-accepted',
        reused: false,
        submissionId: request.submissionId,
      }),
    );
  }

  reconcile(submissionId: string) {
    return Promise.resolve(
      ok({
        kind: 'published' as const,
        matrixEventId: '$fixture-accepted',
        reused: true,
        submissionId,
      }),
    );
  }

  resolveIdentity() {
    return Promise.resolve(
      ok({
        agentId: '01990d9e-8400-7000-8000-000000000001',
        displayName: 'Build Agent',
        instanceId: '01990d9e-8400-7000-8000-000000000002',
        matrixUserId: '@build-agent:agent-room.test',
        provenance: 'human_confirmed_agent' as const,
        source: 'bridge_agent_instance' as const,
      }),
    );
  }
}

const services: AppServices = {
  config: {
    controlPlaneUrl: 'https://api.agent-room.test',
    matrixHomeserverUrl: 'https://matrix.agent-room.test',
  },
  content,
  contentVerifier,
  controlPlane: new ControlPlaneClient({ baseUrl: 'https://api.agent-room.test' }),
  lobby,
  messagePublisher: new FixtureMessagePublisher(),
  messages,
};

function LobbyFixture() {
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);
  return (
    <I18nextProvider i18n={i18n}>
      <AppServicesProvider services={services}>
        <LobbyPage
          catalogId="public-builders"
          onSelectedAgentChange={setSelectedAgentId}
          onSelectedMessageChange={setSelectedMessageId}
          roomId={room.roomId}
          selectedAgentId={selectedAgentId}
          selectedMessageId={selectedMessageId}
        />
      </AppServicesProvider>
    </I18nextProvider>
  );
}

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage, ['en']);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('大厅测试根节点不存在。');
  }
  createRoot(root).render(<LobbyFixture />);
}

function testRoom(agentCount: number): LobbyRoom {
  return Object.freeze({
    agents: Object.freeze(Array.from({ length: agentCount }, (_, index) => testAgent(index))),
    name: 'Builders Exchange',
    observedAtUnixMs: Date.now(),
    roomId: '!builders:agent-room.test',
    topic: 'Live coordination across verified local Agent instances',
  });
}

function testAgent(index: number): LobbyAgent {
  const suffix = String(index + 1).padStart(3, '0');
  const status = statusAt(index);
  const detailed = index % 3 !== 0;
  return Object.freeze({
    agentId: `agent-${suffix}`,
    displayName: `Build Agent ${suffix}`,
    instanceIds: Object.freeze(
      index % 7 === 0 ? [`instance-${suffix}-a`, `instance-${suffix}-b`] : [`instance-${suffix}`],
    ),
    matrixUserId: `@build-agent-${suffix}:agent-room.test`,
    status,
    statusExpiresAtUnixMs: Date.now() + 300_000,
    ...(detailed ? { summary: `Validating workspace slice ${suffix}` } : {}),
    trust: index % 5 === 0 ? 'verified' : 'unknown',
    visibility: detailed ? 'detailed' : 'coarse',
  });
}

function statusAt(index: number): LobbyAgentStatus {
  const statuses: readonly LobbyAgentStatus[] = [
    'working',
    'working',
    'idle',
    'waiting_input',
    'blocked',
    'completed',
    'offline',
  ];
  return statuses[index % statuses.length] ?? 'offline';
}

function noop(): void {
  return undefined;
}

void bootstrapFixture();
