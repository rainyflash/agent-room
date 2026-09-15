import { useState, useSyncExternalStore } from 'react';
import { I18nextProvider } from 'react-i18next';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useRouterState } from '@tanstack/react-router';
import { AppServicesProvider, type AppServices } from '@/app/app-services';
import { i18n } from '@/shared/i18n/i18n';
import { SessionProvider } from '@/features/session/ui/session-provider';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { ConversationWorkspaceProvider } from '@/features/conversation/ui/conversation-workspace-context';
import { ConversationPanel } from '@/features/conversation/ui/conversation-panel';
import { RoomDirectoryPage } from '@/features/room-directory/ui/room-directory-page';
import { HallActions } from '@/features/room-directory/ui/hall-actions';
import { ApplicationAboutPage } from '@/features/updates/ui/application-about-page';
import { AppNavigation } from '@/shared/ui/app-navigation';
import { BrowserMessageBodyPreparer } from '@/features/messages/adapters/browser-message-body-preparer';
import { BrowserMessageSubmissionJournal } from '@/features/messages/adapters/browser-message-submission-journal';
import { BrowserContentVerifier } from '@/features/messages/adapters/browser-content-verifier';
import { HumanMessagePublisher } from '@/features/messages/application/human-message-publisher';
import { encryptContent } from '@/features/messages/adapters/browser-content-cipher';
import type { PrivateRoom, PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import type {
  HumanMessagePreviewEvent,
  MessageContentUploadRequest,
  MessagePublisher,
} from '@/features/messages/domain/publication';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { WebSession } from '@/features/session/domain/session';
import { ok, err } from '@/shared/result';

// Isolated UI acceptance harness: real publication, cryptography and IndexedDB; in-memory services.
const principal: WebSession = {
  principalId: '01990d9e-8400-7000-8000-000000000001',
  matrixUserId: '@features:matrix.test',
  displayName: 'Feature test user',
  locale: 'en',
  authenticatedAtUnixMs: 1,
  expiresAtUnixMs: 9_999_999_999_999,
  recentlyAuthenticated: false,
};
const firstCatalog = '01990d9e-8400-7000-8000-000000000401';
const publicCatalog = '01990d9e-8400-7000-8000-000000000499';
function makeRoom(catalogId: string, name: string): PrivateRoom {
  return {
    catalogId,
    name,
    description: 'A shared hall for a project',
    matrixRoomId: `!${catalogId}:matrix.test`,
    roomInstanceId: catalogId,
    ownerPrincipalId: principal.principalId,
    retentionDays: null,
    status: 'active',
    version: 1,
    members: [
      {
        principalId: principal.principalId,
        status: 'joined',
        permissions: { capabilities: ['view', 'speak', 'invite', 'manage'] },
      },
    ],
  };
}
const seededRooms = [
  makeRoom(firstCatalog, 'Design studio'),
  makeRoom('01990d9e-8400-7000-8000-000000000402', 'Engineering hall'),
];
const rooms = new Map(seededRooms.map((room) => [room.catalogId, room]));
const uploads = new Map<string, MessageContentUploadRequest>();
const transactions = new Map<string, string>();
const listeners = new Set<() => void>();
let messages: readonly RoomMessageSignal[] = [];
const counters = {
  creates: [] as string[],
  uploads: [] as { submissionId: string; digest: string }[],
  publishes: 0,
  binds: 0,
};
Object.defineProperty(window, '__applicationFeatureEvidence', {
  configurable: true,
  get: () => counters,
});
const params = new URLSearchParams(window.location.search);
let failJoin = params.has('failJoin');
let failUpload = params.has('failUpload');

function append(event: HumanMessagePreviewEvent) {
  if (messages.some((message) => message.messageId === event.id)) return;
  messages = [
    ...messages,
    {
      actor: { ...event.actor, provenance: 'human' },
      content: event.content,
      edited: false,
      endToEndEncrypted: true,
      lifecycle: 'active',
      matrixEventId: `$${event.id}`,
      messageId: event.id,
      preview: event.preview,
      roomId: event.roomId,
      serverTimestamp: Date.now(),
      signatureStatus: 'matrix_sender_matched',
      ...(event.relation === undefined ? {} : { relation: event.relation }),
    },
  ];
  for (const listener of listeners) listener();
}
function featureServices(base: AppServices): AppServices {
  const unavailable = () => Promise.resolve(err({ code: 'fixture.unavailable', retryable: false }));
  const privateRooms: PrivateRoomGateway = {
    ...base.privateRooms,
    create: (id, input) => {
      counters.creates.push(id);
      if (!rooms.has(id))
        rooms.set(id, { ...makeRoom(id, input.name), description: input.description });
      const room = rooms.get(id);
      return Promise.resolve(
        room === undefined ? err({ code: 'fixture.missing', retryable: false }) : ok(room),
      );
    },
    list: () => Promise.resolve(ok([...rooms.values()])),
    inspect: (id) => {
      const room = rooms.get(id);
      return Promise.resolve(
        room === undefined ? err({ code: 'fixture.missing', retryable: false }) : ok(room),
      );
    },
  };
  const session = {
    ...base.session,
    controlPlane: {
      readSession: () => Promise.resolve(ok(principal)),
      beginAuthentication: base.session.controlPlane.beginAuthentication.bind(
        base.session.controlPlane,
      ),
      logout: () => Promise.resolve(ok(undefined)),
    },
    matrix: {
      ...base.session.matrix,
      restore: () =>
        Promise.resolve(
          ok({
            kind: 'connected' as const,
            connection: {
              deviceId: 'TEST',
              userId: principal.matrixUserId,
              disconnect: () => undefined,
              observe: () => () => undefined,
              waitUntilPrepared: () => Promise.resolve(ok(undefined)),
            },
          }),
        ),
    },
  };
  const publisher = new HumanMessagePublisher({
    bodyPreparer: new BrowserMessageBodyPreparer(),
    journal: new BrowserMessageSubmissionJournal(localStorage, sessionStorage),
    session: session.controlPlane,
    content: {
      upload: (request) => {
        counters.uploads.push({
          submissionId: request.submissionId,
          digest: request.body.digestSha256,
        });
        if (failUpload) {
          failUpload = false;
          return Promise.resolve(err({ code: 'publication.content_rejected', retryable: true }));
        }
        const old = uploads.get(request.submissionId);
        if (old !== undefined && old.body.digestSha256 !== request.body.digestSha256)
          return Promise.resolve(err({ code: 'publication.content_rejected', retryable: false }));
        uploads.set(request.submissionId, request);
        return Promise.resolve(
          ok({
            contentId: request.submissionId,
            digestSha256: request.body.digestSha256,
            mediaType: request.mediaType,
            sizeBytes: request.body.bytes.byteLength,
          }),
        );
      },
      bind: () => {
        counters.binds += 1;
        return Promise.resolve(ok(undefined));
      },
    },
    matrix: {
      currentUserId: () => principal.matrixUserId,
      protectBody: async (request, body) =>
        ok(await encryptContent(body, request.submissionId, request.roomId, request.mediaType)),
      findByTransaction: (_room, id) => transactions.get(id) ?? null,
      publish: (request) => {
        counters.publishes += 1;
        const matrixEventId = `$${request.event.id}`;
        transactions.set(request.transactionId, matrixEventId);
        append(request.event);
        return Promise.resolve(ok({ matrixEventId }));
      },
    },
  });
  return {
    ...base,
    session,
    privateRooms,
    messagePublisher: publisher,
    privateRoomMatrix: {
      join: () => {
        if (failJoin) {
          failJoin = false;
          return unavailable();
        }
        return Promise.resolve(ok(undefined));
      },
      leave: () => Promise.resolve(ok(undefined)),
    },
    roomDirectory: {
      list: () =>
        Promise.resolve(
          ok([
            {
              catalogId: publicCatalog,
              name: 'Global hall',
              description: 'Meet people and agents',
              slug: 'global',
              language: 'en',
              activeInstanceCount: 1,
              onlineAgentCount: 8,
            },
          ]),
        ),
    },
    content: {
      issueReadTicket: () =>
        Promise.resolve(ok({ expiresAtUnixMs: Date.now() + 60000, ticket: 'test-ticket' })),
      download: (id) => {
        const upload = uploads.get(id);
        if (upload === undefined)
          return Promise.resolve(err({ code: 'content.download_rejected', retryable: false }));
        const digest = Uint8Array.from(upload.body.digestSha256.match(/../gu) ?? [], (hex) =>
          Number.parseInt(hex, 16),
        );
        return Promise.resolve(
          ok({
            bytes: upload.body.bytes,
            contentLength: String(upload.body.bytes.byteLength),
            mediaType: upload.mediaType,
            contentDigest: `sha-256=:${btoa(String.fromCharCode(...digest))}:`,
          }),
        );
      },
    },
    contentVerifier: new BrowserContentVerifier(),
  };
}

export function ApplicationFeaturesFixture({ base }: { readonly base: AppServices }) {
  const [services] = useState(() => featureServices(base));
  const [queries] = useState(
    () => new QueryClient({ defaultOptions: { queries: { retry: false } } }),
  );
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const match = /\/instance\/(.+)$/u.exec(pathname);
  const current =
    [...rooms.values()].find(
      (room) => room.matrixRoomId === decodeURIComponent(match?.[1] ?? ''),
    ) ?? seededRooms[0];
  if (current === undefined) throw new Error('Fixture hall is missing');
  return (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queries}>
        <AppServicesProvider services={services}>
          <SessionProvider dependencies={services.session}>
            <DesktopRuntimeProvider gateway={base.localRuntime}>
              <ConversationWorkspaceProvider
                publisher={services.messagePublisher}
                scope={principal.matrixUserId}
              >
                {pathname === '/about' ? (
                  <ApplicationAboutPage />
                ) : pathname === '/rooms' || pathname.includes('/fixtures/') ? (
                  <RoomDirectoryPage />
                ) : (
                  <main
                    id="main-content"
                    style={{ padding: '0 4vw 2rem', maxWidth: 1100, margin: 'auto' }}
                  >
                    <AppNavigation />
                    <header
                      style={{
                        display: 'flex',
                        gap: 16,
                        flexWrap: 'wrap',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        margin: '24px 0',
                      }}
                    >
                      <h1>{current.name}</h1>
                      <HallActions currentCatalogId={current.catalogId} />
                    </header>
                    <FeatureConversation room={current} publisher={services.messagePublisher} />
                  </main>
                )}
              </ConversationWorkspaceProvider>
            </DesktopRuntimeProvider>
          </SessionProvider>
        </AppServicesProvider>
      </QueryClientProvider>
    </I18nextProvider>
  );
}
function FeatureConversation({
  room,
  publisher,
}: {
  readonly room: PrivateRoom;
  readonly publisher: MessagePublisher;
}) {
  const timeline = useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    () => messages,
  );
  return (
    <div style={{ height: 'min(720px, 78dvh)', minHeight: 500 }}>
      <ConversationPanel
        roomId={room.matrixRoomId}
        roomName={room.name}
        publisher={publisher}
        messages={timeline.filter((message) => message.roomId === room.matrixRoomId)}
        state="ready"
        participants={[{ matrixUserId: '@agent:matrix.test', displayName: 'Test agent' }]}
      />
    </div>
  );
}
