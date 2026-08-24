import { assign, fromCallback, fromPromise, setup } from 'xstate';

import {
  type MessagePublicationFailure,
  type MessagePublicationOutcome,
  type MessagePublicationRequest,
  type MessagePublicationResult,
  type MessagePublisher,
  type MessagePublisherIdentity,
  type PublicationProgressStage,
  validatePublicationRequest,
} from '@/features/messages/domain/publication';
import { err, type Result } from '@/shared/result';

export type MessagePublicationContext = {
  readonly failure: MessagePublicationFailure | null;
  readonly identity: MessagePublisherIdentity | null;
  readonly outcome: MessagePublicationOutcome | null;
  readonly progress: PublicationProgressStage | null;
  readonly recovery: 'none' | 'publish' | 'reconcile';
  readonly request: MessagePublicationRequest | null;
  readonly roomId: string | null;
};

export type MessagePublicationEvent =
  | { readonly roomId: string; readonly type: 'OPEN' }
  | { readonly type: 'CLOSE' }
  | { readonly request: MessagePublicationRequest; readonly type: 'SUBMIT' }
  | { readonly type: 'RETRY' }
  | { readonly type: 'RETRY_IDENTITY' }
  | { readonly type: 'RECONCILE' }
  | { readonly type: 'RESET' }
  | { readonly stage: PublicationProgressStage; readonly type: 'PUBLICATION_PROGRESS' }
  | { readonly submissionId: string; readonly type: 'PUBLICATION_INTERRUPTED' }
  | { readonly result: MessagePublicationResult; readonly type: 'PUBLICATION_RESOLVED' };

const unexpectedFailure: MessagePublicationFailure = Object.freeze({
  code: 'publication.unexpected_failure',
  retryable: false,
});

type PublicationWorkerInput = {
  readonly publisher: MessagePublisher;
  readonly request: MessagePublicationRequest | null;
};

type PublicationWorkerCommand = { readonly type: 'STOP' };

export function createMessagePublicationMachine(publisher: MessagePublisher) {
  const resolveIdentity = fromPromise<Result<MessagePublisherIdentity, MessagePublicationFailure>>(
    async () => await publisher.resolveIdentity(),
  );
  const publishMessage = fromCallback<PublicationWorkerCommand, PublicationWorkerInput>(
    ({ input, sendBack }) => {
      let active = true;
      const request = input.request;
      if (request === null) {
        sendBack({ result: err(unexpectedFailure), type: 'PUBLICATION_RESOLVED' });
        return () => {
          active = false;
        };
      }
      void input.publisher
        .publish(request, (stage) => {
          if (active) {
            sendBack({ stage, type: 'PUBLICATION_PROGRESS' });
          }
        })
        .then((result) => {
          if (active) {
            sendBack({ result, type: 'PUBLICATION_RESOLVED' });
          }
        })
        .catch(() => {
          if (active) {
            sendBack({ submissionId: request.submissionId, type: 'PUBLICATION_INTERRUPTED' });
          }
        });
      return () => {
        active = false;
      };
    },
  );
  const reconcileMessage = fromPromise<
    MessagePublicationResult,
    { readonly request: MessagePublicationRequest | null }
  >(async ({ input }) => {
    return input.request === null
      ? err(unexpectedFailure)
      : await publisher.reconcile(input.request.submissionId);
  });

  return setup({
    types: {
      context: {} as MessagePublicationContext,
      events: {} as MessagePublicationEvent,
    },
    actors: { publishMessage, reconcileMessage, resolveIdentity },
    guards: {
      canCloseFailure: ({ context }) => context.recovery !== 'reconcile',
      canRetryPublication: ({ context }) => context.recovery === 'publish',
      canRetryReconciliation: ({ context }) => context.recovery === 'reconcile',
      requestIsValid: ({ context, event }) =>
        event.type === 'SUBMIT' &&
        event.request.roomId === context.roomId &&
        validatePublicationRequest(event.request).length === 0,
      resolvedAsBindingPending: ({ event }) =>
        event.type === 'PUBLICATION_RESOLVED' &&
        event.result.ok &&
        event.result.value.kind === 'accepted_binding_pending',
      resolvedAsPublished: ({ event }) =>
        event.type === 'PUBLICATION_RESOLVED' &&
        event.result.ok &&
        event.result.value.kind === 'published',
      resolvedAsUnknown: ({ event }) =>
        event.type === 'PUBLICATION_RESOLVED' &&
        event.result.ok &&
        event.result.value.kind === 'pending_reconciliation',
    },
    actions: {
      clearSession: assign({
        failure: null,
        identity: null,
        outcome: null,
        progress: null,
        recovery: 'none' as const,
        request: null,
        roomId: null,
      }),
      clearSubmission: assign({
        failure: null,
        outcome: null,
        progress: null,
        recovery: 'none' as const,
        request: null,
      }),
      setInvalidIntent: assign({
        failure: {
          code: 'publication.invalid_intent',
          retryable: false,
        } satisfies MessagePublicationFailure,
        outcome: null,
        progress: null,
        recovery: 'none' as const,
        request: null,
      }),
      setPublicationFailure: assign({
        failure: ({ event }) =>
          event.type === 'PUBLICATION_RESOLVED' && !event.result.ok
            ? event.result.error
            : unexpectedFailure,
        outcome: null,
        progress: null,
        recovery: ({ event }) =>
          event.type === 'PUBLICATION_RESOLVED' && !event.result.ok && event.result.error.retryable
            ? 'publish'
            : 'none',
      }),
      setUnexpectedFailure: assign({
        failure: unexpectedFailure,
        progress: null,
        recovery: 'none' as const,
      }),
    },
  }).createMachine({
    id: 'message-publication',
    initial: 'closed',
    context: {
      failure: null,
      identity: null,
      outcome: null,
      progress: null,
      recovery: 'none',
      request: null,
      roomId: null,
    },
    states: {
      closed: {
        on: {
          OPEN: {
            actions: assign({ roomId: ({ event }) => event.roomId }),
            target: 'resolvingIdentity',
          },
        },
      },
      resolvingIdentity: {
        invoke: {
          id: 'resolve-publisher-identity',
          src: 'resolveIdentity',
          onDone: [
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                failure: null,
                identity: ({ event }) => (event.output.ok ? event.output.value : null),
              }),
              target: 'ready',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                identity: null,
              }),
              target: 'identityUnavailable',
            },
          ],
          onError: { actions: 'setUnexpectedFailure', target: 'identityUnavailable' },
        },
      },
      identityUnavailable: {
        on: {
          CLOSE: { actions: 'clearSession', target: 'closed' },
          RETRY_IDENTITY: { target: 'resolvingIdentity' },
        },
      },
      ready: {
        on: {
          CLOSE: { actions: 'clearSession', target: 'closed' },
          SUBMIT: [
            {
              guard: 'requestIsValid',
              actions: assign({
                failure: null,
                outcome: null,
                progress: 'uploading' as const,
                recovery: 'none' as const,
                request: ({ event }) => event.request,
              }),
              target: 'publishing',
            },
            { actions: 'setInvalidIntent', target: 'failed' },
          ],
        },
      },
      publishing: {
        invoke: {
          id: 'publish-message',
          src: 'publishMessage',
          input: ({ context }) => ({ publisher, request: context.request }),
        },
        on: {
          PUBLICATION_PROGRESS: {
            actions: assign({ progress: ({ event }) => event.stage }),
          },
          PUBLICATION_INTERRUPTED: {
            actions: assign({
              failure: null,
              outcome: ({ event }) => ({
                kind: 'pending_reconciliation' as const,
                submissionId: event.submissionId,
                transactionId: `agent-room-message-${event.submissionId}`,
              }),
              progress: null,
              recovery: 'none' as const,
            }),
            target: 'unknown',
          },
          PUBLICATION_RESOLVED: [
            {
              guard: 'resolvedAsPublished',
              actions: assign({
                failure: null,
                outcome: ({ event }) => (event.result.ok ? event.result.value : null),
                progress: null,
              }),
              target: 'published',
            },
            {
              guard: 'resolvedAsBindingPending',
              actions: assign({
                failure: null,
                outcome: ({ event }) => (event.result.ok ? event.result.value : null),
                progress: null,
              }),
              target: 'acceptedBindingPending',
            },
            {
              guard: 'resolvedAsUnknown',
              actions: assign({
                failure: null,
                outcome: ({ event }) => (event.result.ok ? event.result.value : null),
                progress: null,
              }),
              target: 'unknown',
            },
            { actions: 'setPublicationFailure', target: 'failed' },
          ],
        },
      },
      acceptedBindingPending: {
        on: { RECONCILE: { target: 'reconciling' } },
      },
      unknown: {
        on: { RECONCILE: { target: 'reconciling' } },
      },
      reconciling: {
        invoke: {
          id: 'reconcile-message',
          src: 'reconcileMessage',
          input: ({ context }) => ({ request: context.request }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok && event.output.value.kind === 'published',
              actions: assign({
                failure: null,
                outcome: ({ event }) => (event.output.ok ? event.output.value : null),
                recovery: 'none' as const,
              }),
              target: 'published',
            },
            {
              guard: ({ event }) =>
                event.output.ok && event.output.value.kind === 'accepted_binding_pending',
              actions: assign({
                failure: null,
                outcome: ({ event }) => (event.output.ok ? event.output.value : null),
                recovery: 'none' as const,
              }),
              target: 'acceptedBindingPending',
            },
            {
              guard: ({ event }) =>
                event.output.ok && event.output.value.kind === 'pending_reconciliation',
              actions: assign({
                failure: null,
                outcome: ({ event }) => (event.output.ok ? event.output.value : null),
                recovery: 'none' as const,
              }),
              target: 'unknown',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                progress: null,
                recovery: ({ event }) =>
                  !event.output.ok && event.output.error.retryable ? 'reconcile' : 'none',
              }),
              target: 'failed',
            },
          ],
          onError: {
            actions: assign({
              failure: { ...unexpectedFailure, retryable: true },
              progress: null,
              recovery: 'reconcile' as const,
            }),
            target: 'failed',
          },
        },
      },
      failed: {
        on: {
          CLOSE: {
            guard: 'canCloseFailure',
            actions: 'clearSession',
            target: 'closed',
          },
          RETRY: [
            { guard: 'canRetryPublication', target: 'publishing' },
            { guard: 'canRetryReconciliation', target: 'reconciling' },
          ],
        },
      },
      published: {
        on: {
          CLOSE: { actions: 'clearSession', target: 'closed' },
          RESET: { actions: 'clearSubmission', target: 'ready' },
        },
      },
    },
  });
}

export type MessagePublicationMachine = ReturnType<typeof createMessagePublicationMachine>;
