import { createRoot, type Root } from 'react-dom/client';
import { useState } from 'react';
import { I18nextProvider } from 'react-i18next';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import type {
  AutomationGrant,
  AutomationGrantGateway,
} from '@/features/automation/domain/automation-grant';
import { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import { DirectSessionCoordinator } from '@/features/direct-sessions/application/direct-session-coordinator';
import type {
  DirectAgent,
  DirectSession,
  DirectSessionGateway,
  DirectSessionMatrixGateway,
} from '@/features/direct-sessions/domain/direct-session';
import type {
  HandoffApprovalRequest,
  HandoffGateway,
  HandoffSnapshot,
} from '@/features/handoffs/domain/handoff';
import type {
  LobbyAgent,
  LobbyAgentStatus,
  LobbyGateway,
  LobbyRoom,
} from '@/features/lobby/domain/lobby';
import { LobbyPage } from '@/features/lobby/ui/lobby-page';
import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { MessageGateway, RoomMessageSignal } from '@/features/messages/domain/message';
import type {
  MessagePublicationRequest,
  MessagePublisher,
  PublicationProgressStage,
} from '@/features/messages/domain/publication';
import type {
  ModerationAction,
  ModerationCase,
  ModerationGateway,
} from '@/features/moderation/domain/moderation';
import type {
  PrivateRoomGateway,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';
import { AccountPreferencesStore } from '@/features/preferences/application/account-preferences-store';
import type { AccountPreferencesGateway } from '@/features/preferences/domain/account-preferences-gateway';
import { AccountPreferencesProvider } from '@/features/preferences/ui/account-preferences-provider';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type { MatrixSecurityGateway } from '@/features/security/domain/matrix-security';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const room = testRoom(200);
const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
const localPreferencesGateway: AccountPreferencesGateway = {
  read: async () => err({ code: 'preferences.source_unavailable', retryable: true }),
  scope: () => null,
  subscribe: () => noop,
  write: async () => err({ code: 'preferences.source_unavailable', retryable: true }),
};
const accountPreferences = new AccountPreferencesStore(localPreferencesGateway, {
  language: 'system',
  lobbyView: 'scene',
});
let fixtureRoot: Root | null = null;
const lobby: LobbyGateway = {
  read: () => ok(room),
  subscribe: () => noop,
};
const messages: MessageGateway = {
  read: (requestedRoomId) =>
    ok({
      messages: testMessages(requestedRoomId),
      observedAtUnixMs: Date.now(),
      roomId: requestedRoomId,
    }),
  subscribe: () => noop,
};
let fixtureModerationCases: readonly ModerationCase[] = Object.freeze([]);
let fixtureModerationActions: readonly ModerationAction[] = Object.freeze([]);
const moderation: ModerationGateway = {
  applyAction: async (actionId, roomCatalogId, input) => {
    const action: ModerationAction = Object.freeze({
      actionId,
      actorPrincipalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
      caseId: input.caseId ?? null,
      expiresAtUnixMs: input.expiresAtUnixMs ?? null,
      failureCode: null,
      kind: input.kind,
      reason: input.reason,
      reversedAtUnixMs: null,
      roomCatalogId,
      startsAtUnixMs: Date.now(),
      status: 'applied',
      targetKind: input.targetKind,
      targetReference: input.targetReference,
    });
    fixtureModerationActions = Object.freeze([action, ...fixtureModerationActions]);
    return ok(action);
  },
  listActions: async (roomCatalogId) =>
    ok(fixtureModerationActions.filter((action) => action.roomCatalogId === roomCatalogId)),
  listAudit: async () => err({ code: 'moderation.forbidden', retryable: false }),
  listCases: async () => ok(fixtureModerationCases),
  listRoomCases: async (roomCatalogId) =>
    ok(
      fixtureModerationCases.filter(
        (moderationCase) => moderationCase.evidence.roomCatalogId === roomCatalogId,
      ),
    ),
  report: async (caseId, input) => {
    const moderationCase: ModerationCase = Object.freeze({
      caseId,
      createdAtUnixMs: Date.now(),
      description: input.description,
      evidence: Object.freeze({
        endToEndEncrypted: input.evidence.endToEndEncrypted,
        matrixEventId: input.evidence.matrixEventId ?? null,
        reporterSubmittedExcerpt: input.evidence.reporterSubmittedExcerpt ?? null,
        roomCatalogId: input.evidence.roomCatalogId ?? null,
      }),
      reason: input.reason,
      resolvedAtUnixMs: null,
      state: 'open',
      targetKind: input.targetKind,
      targetReference: input.targetReference,
    });
    fixtureModerationCases = Object.freeze([moderationCase, ...fixtureModerationCases]);
    return ok(moderationCase);
  },
  reverseAction: async (actionId) => {
    const action = fixtureModerationActions.find((candidate) => candidate.actionId === actionId);
    if (action === undefined) {
      return err({ code: 'moderation.action_not_found', retryable: false });
    }
    const reversed = Object.freeze({
      ...action,
      reversedAtUnixMs: Date.now(),
      status: 'reversed' as const,
    });
    fixtureModerationActions = Object.freeze(
      fixtureModerationActions.map((candidate) =>
        candidate.actionId === actionId ? reversed : candidate,
      ),
    );
    return ok(reversed);
  },
};
const unavailablePrivateRoom = async () =>
  err({ code: 'private_room.fixture_unavailable', retryable: false });
const privateRooms: PrivateRoomGateway = {
  accept: unavailablePrivateRoom,
  archive: unavailablePrivateRoom,
  ban: unavailablePrivateRoom,
  create: unavailablePrivateRoom,
  decline: unavailablePrivateRoom,
  inspect: unavailablePrivateRoom,
  invite: unavailablePrivateRoom,
  leave: unavailablePrivateRoom,
  list: async () => ok([]),
  remove: unavailablePrivateRoom,
  transferOwnership: unavailablePrivateRoom,
  updatePermissions: unavailablePrivateRoom,
};
const privateRoomMatrix: PrivateRoomMatrixGateway = {
  join: unavailablePrivateRoom,
  leave: unavailablePrivateRoom,
};
const accessManagement: AccessManagementGateway = {
  listAgentInstances: async () =>
    ok([
      {
        adapterType: 'codex',
        agentAvatarContentId: null,
        agentDisplayName: 'Fixture Codex Agent',
        agentId: '01990d9e-8400-7000-8000-000000000001',
        agentInstanceId: '01990d9e-8400-7000-8000-000000000201',
        capabilityVersion: '1',
        createdAtUnixMs: Date.now() - 60_000,
        device: {
          deviceId: '01990d9e-8400-7000-8000-000000000301',
          label: 'Fixture workstation',
          platform: 'windows',
          trustState: 'verified',
        },
        lastSeenAtUnixMs: Date.now(),
        matrixDeviceId: 'FIXTURE-MATRIX-DEVICE',
        matrixDeviceRevokedAtUnixMs: null,
        revokedAtUnixMs: null,
        status: 'online',
      },
    ]),
  listProductDevices: async () => ok([]),
  revokeAgentInstance: async () => err({ code: 'access.fixture_unavailable', retryable: false }),
  revokeProductDevice: async () => err({ code: 'access.fixture_unavailable', retryable: false }),
};
let fixtureAutomationGrants: readonly AutomationGrant[] = Object.freeze([]);
const automation: AutomationGrantGateway = {
  create: async (grantId, input) => {
    const startsAtUnixMs = Date.now();
    const grant: AutomationGrant = Object.freeze({
      agentId: input.agentId,
      agentInstanceId: input.agentInstanceId ?? null,
      audience: input.audience,
      expiresAtUnixMs: startsAtUnixMs + input.lifetimeSeconds * 1_000,
      grantId,
      maxMessagesPerMinute: input.maxMessagesPerMinute,
      maxTotalMessages: input.maxTotalMessages ?? null,
      messageKinds: Object.freeze([...input.messageKinds]),
      messagesInCurrentMinute: 0,
      requiresRiskScan: input.requiresRiskScan,
      revokedAtUnixMs: null,
      roomCatalogId: input.roomCatalogId,
      startsAtUnixMs,
      status: 'active',
      totalMessages: 0,
    });
    fixtureAutomationGrants = Object.freeze([grant, ...fixtureAutomationGrants]);
    return ok(grant);
  },
  list: async () => ok(fixtureAutomationGrants),
  revoke: async (grantId) => {
    const grant = fixtureAutomationGrants.find((candidate) => candidate.grantId === grantId);
    if (grant === undefined) {
      return err({ code: 'automation.fixture_not_found', retryable: false });
    }
    const revoked = Object.freeze({
      ...grant,
      revokedAtUnixMs: Date.now(),
      status: 'revoked' as const,
    });
    fixtureAutomationGrants = Object.freeze(
      fixtureAutomationGrants.map((candidate) =>
        candidate.grantId === grantId ? revoked : candidate,
      ),
    );
    return ok(revoked);
  },
};
const security: MatrixSecurityGateway = {
  acceptIncomingVerification: async () =>
    err({ code: 'security.matrix_unavailable', retryable: true }),
  beginVerification: async () => err({ code: 'security.matrix_unavailable', retryable: true }),
  declineIncomingVerification: async () =>
    err({ code: 'security.matrix_unavailable', retryable: true }),
  establishIdentity: async () => err({ code: 'security.matrix_unavailable', retryable: true }),
  getIncomingVerification: () => null,
  inspect: async () => err({ code: 'security.matrix_unavailable', retryable: true }),
  recover: async () => err({ code: 'security.matrix_unavailable', retryable: true }),
  setupRecovery: async () => err({ code: 'security.matrix_unavailable', retryable: true }),
  subscribe: () => noop,
};
let fixtureDirectSessions: readonly DirectSession[] = Object.freeze([testDirectSession(1)]);
const directSessions: DirectSessionGateway = {
  inspect: async (catalogId) => {
    const session = fixtureDirectSessions.find((candidate) => candidate.catalogId === catalogId);
    return session === undefined
      ? err({ code: 'direct_session.fixture_not_found', retryable: false })
      : ok(session);
  },
  list: async () => ok(fixtureDirectSessions),
  open: async (targetAgentId) => {
    const index = room.agents.findIndex((agent) => agent.agentId === targetAgentId);
    if (index < 0) {
      return err({ code: 'direct_session.fixture_target_not_found', retryable: false });
    }
    const existing = fixtureDirectSessions.find(
      (session) => session.target.agentId === targetAgentId,
    );
    if (existing !== undefined) {
      return ok(existing);
    }
    const opened = testDirectSession(index);
    fixtureDirectSessions = Object.freeze([opened, ...fixtureDirectSessions]);
    return ok(opened);
  },
  setBlocked: async (targetAgentId, blocked) => {
    const lobbyAgent = room.agents.find((agent) => agent.agentId === targetAgentId);
    if (lobbyAgent === undefined) {
      return err({ code: 'direct_session.fixture_target_not_found', retryable: false });
    }
    const contactPolicy = Object.freeze({
      agentBlocksPrincipal: false,
      deliveryAllowed: !blocked,
      presenceDisclosure: blocked ? ('hidden' as const) : ('coarse' as const),
      principalBlocksAgent: blocked,
    });
    fixtureDirectSessions = Object.freeze(
      fixtureDirectSessions.map((session) =>
        session.target.agentId === targetAgentId
          ? Object.freeze({ ...session, contactPolicy, version: session.version + 1 })
          : session,
      ),
    );
    return ok({ contactPolicy, target: toFixtureDirectAgent(lobbyAgent) });
  },
};
const directSessionMatrix: DirectSessionMatrixGateway = {
  markDisplayed: async () => ok(undefined),
  prepare: async () => ok(undefined),
  setIgnored: async () => ok(undefined),
};
const locallyBlockedAgents = new Set<string>();
const directSessionCoordinator = new DirectSessionCoordinator(directSessions, directSessionMatrix, {
  has: (agentId) => locallyBlockedAgents.has(agentId),
  set: (agentId, blocked) => {
    if (blocked) {
      locallyBlockedAgents.add(agentId);
    } else {
      locallyBlockedAgents.delete(agentId);
    }
  },
});
const fixtureContent =
  '# Protocol review\n<img src=x onerror=alert(1)>\n[link](javascript:alert(1))';
const contentReadCounts = { downloads: 0, tickets: 0 };
Object.defineProperty(window, '__agentRoomFixtureContentReads', {
  configurable: true,
  get: () => ({ ...contentReadCounts }),
});
const content: ContentGateway = {
  download: () => {
    contentReadCounts.downloads += 1;
    return Promise.resolve(
      ok({
        bytes: new TextEncoder().encode(fixtureContent),
        contentDigest: `sha-256=:${'q'.repeat(43)}=:`,
        contentLength: String(new TextEncoder().encode(fixtureContent).byteLength),
        mediaType: 'text/markdown',
      }),
    );
  },
  issueReadTicket: () => {
    contentReadCounts.tickets += 1;
    return Promise.resolve(
      ok({ expiresAtUnixMs: Date.now() + 60_000, ticket: 'fixture-read-ticket' }),
    );
  },
};
const contentVerifier: ContentVerifier = {
  verify: (downloaded, expected) =>
    Promise.resolve(
      ok({
        bytes: downloaded.bytes,
        digestSha256: expected.digestSha256,
        mediaType: expected.mediaType,
        mode: 'text',
        text: fixtureContent,
      }),
    ),
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

class FixtureHandoffGateway implements HandoffGateway {
  #snapshot: HandoffSnapshot | null = null;

  approve(request: HandoffApprovalRequest) {
    this.#snapshot = {
      expiresAtUnixMs: request.expiresAtUnixMs,
      handoffId: request.handoffId,
      status: 'approved',
    };
    return Promise.resolve(
      ok({ handoffId: request.handoffId, kind: 'submitted' as const, reused: false }),
    );
  }

  listTargets() {
    return Promise.resolve(
      ok([
        {
          agentId: '01990d9e-8400-7000-8000-000000000011',
          displayName: 'Local Codex Agent',
          instanceId: '01990d9e-8400-7000-8000-000000000012',
        },
      ]),
    );
  }

  reconcile() {
    const snapshot = this.#snapshot;
    return Promise.resolve(
      snapshot === null
        ? err({ code: 'handoff.not_found' as const, retryable: false })
        : ok({ ...snapshot, status: 'delivered' as const }),
    );
  }

  revoke() {
    const snapshot = this.#snapshot;
    return Promise.resolve(
      snapshot === null
        ? err({ code: 'handoff.not_found' as const, retryable: false })
        : ok({ ...snapshot, status: 'revoked' as const }),
    );
  }
}

const services: AppServices = {
  accessManagement,
  automation,
  config: {
    controlPlaneUrl: 'https://api.agent-room.test',
    matrixHomeserverUrl: 'https://matrix.agent-room.test',
  },
  content,
  contentVerifier,
  controlPlane: new ControlPlaneClient({ baseUrl: 'https://api.agent-room.test' }),
  directSessionCoordinator,
  directSessions,
  handoffs: new FixtureHandoffGateway(),
  lobby,
  messagePublisher: new FixtureMessagePublisher(),
  messages,
  moderation,
  privateRoomMatrix,
  privateRooms,
  security,
};

function LobbyFixture() {
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [selectedDirectSessionId, setSelectedDirectSessionId] = useState<string | null>(null);
  const [selectedMessageId, setSelectedMessageId] = useState<string | null>(null);
  return (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <AppServicesProvider services={services}>
          <AccountPreferencesProvider store={accountPreferences}>
            <LobbyPage
              catalogId="01990d9e-8400-7000-8000-000000000401"
              onEnterRoom={() => undefined}
              onExitRoom={() => undefined}
              onOpenSecurity={() => undefined}
              onSelectedAgentChange={setSelectedAgentId}
              onSelectedDirectSessionChange={setSelectedDirectSessionId}
              onSelectedMessageChange={setSelectedMessageId}
              principal={{
                authenticatedAtUnixMs: Date.now(),
                displayName: 'Fixture operator',
                expiresAtUnixMs: Date.now() + 60_000,
                locale: 'en',
                matrixUserId: '@fixture:matrix.test',
                principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
                recentlyAuthenticated: true,
              }}
              roomId={room.roomId}
              selectedAgentId={selectedAgentId}
              selectedDirectSessionId={selectedDirectSessionId}
              selectedMessageId={selectedMessageId}
            />
          </AccountPreferencesProvider>
        </AppServicesProvider>
      </QueryClientProvider>
    </I18nextProvider>
  );
}

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage, ['en']);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('大厅测试根节点不存在。');
  }
  fixtureRoot = createRoot(root);
  fixtureRoot.render(<LobbyFixture />);
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
    agentId: `01990d9e-8400-7000-8000-000000000${suffix}`,
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

function testDirectSession(index: number): DirectSession {
  const suffix = String(index + 1).padStart(3, '0');
  const target = room.agents[index];
  if (target === undefined) {
    throw new Error('直接会话夹具引用了不存在的 Agent。');
  }
  return Object.freeze({
    catalogId: `01990d9e-8400-7000-8000-000000001${suffix}`,
    contactPolicy: Object.freeze({
      agentBlocksPrincipal: false,
      deliveryAllowed: true,
      presenceDisclosure: 'coarse',
      principalBlocksAgent: false,
    }),
    lifecycle: 'active',
    matrixRoomId: `!direct-${suffix}:agent-room.test`,
    roomInstanceId: `01990d9e-8400-7000-8000-000000002${suffix}`,
    target: toFixtureDirectAgent(target),
    version: 0,
  });
}

function toFixtureDirectAgent(agent: LobbyAgent): DirectAgent {
  return Object.freeze({
    agentId: agent.agentId,
    avatarContentId: null,
    displayName: agent.displayName,
    matrixUserId: agent.matrixUserId,
  });
}

function testMessages(roomId: string): readonly RoomMessageSignal[] {
  return Object.freeze([
    testMessage(
      roomId,
      '01990d9e-8400-7000-8000-000000000011',
      'Protocol review ready',
      'Open only when you want to inspect the verified bytes.',
      Date.now() - 20_000,
      ['untrusted_instructions'],
    ),
    testMessage(
      roomId,
      '01990d9e-8400-7000-8000-000000000012',
      'Build completed',
      'The current workspace slice passed its local checks.',
      Date.now() - 50_000,
      [],
    ),
  ]);
}

function testMessage(
  roomId: string,
  messageId: string,
  title: string,
  summary: string,
  serverTimestamp: number,
  riskFlags: readonly string[],
): RoomMessageSignal {
  return Object.freeze({
    actor: Object.freeze({
      agentId: '01990d9e-8400-7000-8000-000000000001',
      displayName: 'Build Agent',
      instanceId: '01990d9e-8400-7000-8000-000000000002',
      matrixUserId: '@build-agent:agent-room.test',
      provenance: 'human_confirmed_agent',
    }),
    content: Object.freeze({
      contentId: '01990d9e-8400-7000-8000-000000000016',
      digestSha256: 'ab'.repeat(32),
      mediaType: 'text/markdown',
      sizeBytes: new TextEncoder().encode(fixtureContent).byteLength,
    }),
    edited: false,
    endToEndEncrypted: false,
    lifecycle: 'active',
    matrixEventId: `$fixture-${messageId}`,
    messageId,
    preview: Object.freeze({
      contentType: 'text/markdown',
      riskFlags: Object.freeze([...riskFlags]),
      sensitivity: 'normal',
      summary,
      title,
    }),
    roomId,
    serverTimestamp,
    signatureStatus: 'matrix_sender_matched',
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

if (import.meta.hot !== undefined) {
  import.meta.hot.dispose(() => {
    fixtureRoot?.unmount();
    fixtureRoot = null;
  });
}

void bootstrapFixture();
