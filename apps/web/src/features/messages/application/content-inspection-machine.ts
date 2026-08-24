import { assign, fromPromise, setup } from 'xstate';

import type {
  ContentFailure,
  ContentGateway,
  ContentReadTicket,
  ContentVerifier,
  DownloadedContent,
  VerifiedContent,
} from '@/features/messages/domain/content';
import type { MessageContentReference } from '@/features/messages/domain/message';
import { err, type Result } from '@/shared/result';

export type ContentInspectionRequest = {
  readonly matrixEventId: string;
  readonly messageId: string;
  readonly reference: MessageContentReference;
};

export type ContentInspectionContext = {
  readonly content: VerifiedContent | null;
  readonly downloaded: DownloadedContent | null;
  readonly failure: ContentFailure | null;
  readonly request: ContentInspectionRequest | null;
  readonly ticket: ContentReadTicket | null;
};

export type ContentInspectionEvent =
  | { readonly request: ContentInspectionRequest; readonly type: 'OPEN' }
  | { readonly type: 'CLOSE' }
  | { readonly type: 'RETRY' };

export type ContentInspectionDependencies = {
  readonly content: ContentGateway;
  readonly verifier: ContentVerifier;
};

const unexpectedFailure: ContentFailure = Object.freeze({
  code: 'content.invalid_response',
  retryable: false,
});

export function createContentInspectionMachine(dependencies: ContentInspectionDependencies) {
  const requestTicket = fromPromise<
    Result<ContentReadTicket, ContentFailure>,
    { readonly request: ContentInspectionRequest | null }
  >(async ({ input }) => {
    return input.request === null
      ? err(unexpectedFailure)
      : await dependencies.content.issueReadTicket(input.request.reference.contentId);
  });
  const downloadContent = fromPromise<
    Result<DownloadedContent, ContentFailure>,
    {
      readonly request: ContentInspectionRequest | null;
      readonly ticket: ContentReadTicket | null;
    }
  >(async ({ input }) => {
    return input.request === null || input.ticket === null
      ? err(unexpectedFailure)
      : await dependencies.content.download(input.request.reference.contentId, input.ticket.ticket);
  });
  const verifyContent = fromPromise<
    Result<VerifiedContent, ContentFailure>,
    {
      readonly downloaded: DownloadedContent | null;
      readonly request: ContentInspectionRequest | null;
    }
  >(async ({ input }) => {
    return input.request === null || input.downloaded === null
      ? err(unexpectedFailure)
      : await dependencies.verifier.verify(input.downloaded, input.request.reference);
  });

  return setup({
    types: {
      context: {} as ContentInspectionContext,
      events: {} as ContentInspectionEvent,
    },
    actors: { downloadContent, requestTicket, verifyContent },
    actions: {
      clearInspection: assign({
        content: null,
        downloaded: null,
        failure: null,
        request: null,
        ticket: null,
      }),
      setUnexpectedFailure: assign({ failure: unexpectedFailure }),
    },
  }).createMachine({
    id: 'content-inspection',
    initial: 'idle',
    context: {
      content: null,
      downloaded: null,
      failure: null,
      request: null,
      ticket: null,
    },
    on: {
      CLOSE: { actions: 'clearInspection', target: '.idle' },
      OPEN: {
        actions: assign({
          content: null,
          downloaded: null,
          failure: null,
          request: ({ event }) => event.request,
          ticket: null,
        }),
        target: '.requestingTicket',
      },
    },
    states: {
      idle: {},
      requestingTicket: {
        invoke: {
          id: 'request-content-ticket',
          src: 'requestTicket',
          input: ({ context }) => ({ request: context.request }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                ticket: ({ event }) => (event.output.ok ? event.output.value : null),
              }),
              target: 'downloading',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
              }),
              target: 'failed',
            },
          ],
          onError: { actions: 'setUnexpectedFailure', target: 'failed' },
        },
      },
      downloading: {
        invoke: {
          id: 'download-content-bytes',
          src: 'downloadContent',
          input: ({ context }) => ({ request: context.request, ticket: context.ticket }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                downloaded: ({ event }) => (event.output.ok ? event.output.value : null),
              }),
              target: 'verifying',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
              }),
              target: 'failed',
            },
          ],
          onError: { actions: 'setUnexpectedFailure', target: 'failed' },
        },
      },
      verifying: {
        invoke: {
          id: 'verify-content-integrity',
          src: 'verifyContent',
          input: ({ context }) => ({
            downloaded: context.downloaded,
            request: context.request,
          }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                content: ({ event }) => (event.output.ok ? event.output.value : null),
                downloaded: null,
                failure: null,
                ticket: null,
              }),
              target: 'ready',
            },
            {
              actions: assign({
                downloaded: null,
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                ticket: null,
              }),
              target: 'failed',
            },
          ],
          onError: { actions: 'setUnexpectedFailure', target: 'failed' },
        },
      },
      ready: {},
      failed: {
        on: {
          RETRY: {
            actions: assign({ content: null, downloaded: null, failure: null, ticket: null }),
            target: 'requestingTicket',
          },
        },
      },
    },
  });
}
