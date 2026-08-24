import { assign, fromPromise, setup } from 'xstate';

import {
  type HandoffApprovalRequest,
  type HandoffFailure,
  type HandoffGateway,
  type HandoffSnapshot,
  type HandoffSubmissionOutcome,
  type HandoffTarget,
  isHandoffActive,
  validateHandoffApproval,
} from '@/features/handoffs/domain/handoff';
import { err, type Result } from '@/shared/result';

export type HandoffDeliveryContext = {
  readonly failure: HandoffFailure | null;
  readonly recovery: 'none' | 'targets' | 'approve' | 'reconcile' | 'revoke';
  readonly request: HandoffApprovalRequest | null;
  readonly roomId: string;
  readonly snapshot: HandoffSnapshot | null;
  readonly targets: readonly HandoffTarget[];
};

export type HandoffDeliveryEvent =
  | { readonly request: HandoffApprovalRequest; readonly type: 'SUBMIT' }
  | { readonly type: 'QUERY' }
  | { readonly type: 'RETRY' }
  | { readonly type: 'REVOKE' };

type HandoffDeliveryDependencies = {
  readonly gateway: HandoffGateway;
  readonly now?: () => number;
  readonly roomId: string;
};

const unexpectedFailure: HandoffFailure = Object.freeze({
  code: 'handoff.unexpected_failure',
  retryable: false,
});

export function createHandoffDeliveryMachine({
  gateway,
  now = Date.now,
  roomId,
}: HandoffDeliveryDependencies) {
  const listTargets = fromPromise(async () => await gateway.listTargets(roomId));
  const approve = fromPromise<
    Result<HandoffSubmissionOutcome, HandoffFailure>,
    { readonly request: HandoffApprovalRequest | null }
  >(async ({ input }) =>
    input.request === null ? err(unexpectedFailure) : await gateway.approve(input.request),
  );
  const reconcile = fromPromise<
    Result<HandoffSnapshot, HandoffFailure>,
    { readonly handoffId: string | null }
  >(async ({ input }) =>
    input.handoffId === null ? err(unexpectedFailure) : await gateway.reconcile(input.handoffId),
  );
  const revoke = fromPromise<
    Result<HandoffSnapshot, HandoffFailure>,
    { readonly handoffId: string | null }
  >(async ({ input }) =>
    input.handoffId === null ? err(unexpectedFailure) : await gateway.revoke(input.handoffId),
  );

  return setup({
    types: {
      context: {} as HandoffDeliveryContext,
      events: {} as HandoffDeliveryEvent,
    },
    actors: { approve, listTargets, reconcile, revoke },
    guards: {
      canRetryApprove: ({ context }) => context.recovery === 'approve',
      canRetryReconcile: ({ context }) => context.recovery === 'reconcile',
      canRetryRevoke: ({ context }) => context.recovery === 'revoke',
      canRetryTargets: ({ context }) => context.recovery === 'targets',
      requestIsValid: ({ event }) =>
        event.type === 'SUBMIT' &&
        event.request.source.roomId === roomId &&
        validateHandoffApproval(event.request, now()).length === 0,
    },
    actions: {
      setInvalidIntent: assign({
        failure: {
          code: 'handoff.invalid_intent',
          retryable: false,
        } satisfies HandoffFailure,
        recovery: 'none' as const,
      }),
      setUnexpectedFailure: assign({
        failure: unexpectedFailure,
        recovery: 'none' as const,
      }),
    },
  }).createMachine({
    id: 'handoff-delivery',
    initial: 'resolvingTargets',
    context: {
      failure: null,
      recovery: 'none',
      request: null,
      roomId,
      snapshot: null,
      targets: [],
    },
    states: {
      resolvingTargets: {
        invoke: {
          id: 'list-handoff-targets',
          src: 'listTargets',
          onDone: [
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                targets: ({ event }) =>
                  event.output.ok ? Object.freeze([...event.output.value]) : [],
              }),
              target: 'ready',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                recovery: ({ event }) =>
                  !event.output.ok && event.output.error.retryable
                    ? ('targets' as const)
                    : ('none' as const),
              }),
              target: 'failed',
            },
          ],
          onError: { actions: 'setUnexpectedFailure', target: 'failed' },
        },
      },
      ready: {
        on: {
          SUBMIT: [
            {
              guard: 'requestIsValid',
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                request: ({ event }) => event.request,
                snapshot: null,
              }),
              target: 'submitting',
            },
            { actions: 'setInvalidIntent', target: 'failed' },
          ],
        },
      },
      submitting: {
        invoke: {
          id: 'approve-handoff',
          src: 'approve',
          input: ({ context }) => ({ request: context.request }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok && event.output.value.kind === 'submitted',
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ context, event }) =>
                  event.output.ok
                    ? snapshotFromSubmission(context.request, event.output.value)
                    : null,
              }),
              target: 'active',
            },
            {
              guard: ({ event }) =>
                event.output.ok && event.output.value.kind === 'delivery_uncertain',
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ context, event }) =>
                  event.output.ok
                    ? snapshotFromSubmission(context.request, event.output.value)
                    : null,
              }),
              target: 'uncertain',
            },
            {
              guard: ({ event }) =>
                event.output.ok &&
                event.output.value.kind === 'resolved' &&
                isHandoffActive(event.output.value.snapshot.status),
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ event }) =>
                  event.output.ok && event.output.value.kind === 'resolved'
                    ? event.output.value.snapshot
                    : null,
              }),
              target: 'active',
            },
            {
              guard: ({ event }) => event.output.ok && event.output.value.kind === 'resolved',
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ event }) =>
                  event.output.ok && event.output.value.kind === 'resolved'
                    ? event.output.value.snapshot
                    : null,
              }),
              target: 'resolved',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                recovery: ({ event }) =>
                  !event.output.ok && event.output.error.retryable
                    ? ('approve' as const)
                    : ('none' as const),
              }),
              target: 'failed',
            },
          ],
          onError: {
            actions: assign({
              failure: { ...unexpectedFailure, retryable: true },
              recovery: 'reconcile' as const,
            }),
            target: 'uncertain',
          },
        },
      },
      active: {
        on: {
          QUERY: { target: 'reconciling' },
          REVOKE: { target: 'revoking' },
        },
      },
      uncertain: {
        on: { QUERY: { target: 'reconciling' } },
      },
      reconciling: {
        invoke: {
          id: 'reconcile-handoff',
          src: 'reconcile',
          input: ({ context }) => ({ handoffId: context.request?.handoffId ?? null }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok && isHandoffActive(event.output.value.status),
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ event }) => (event.output.ok ? event.output.value : null),
              }),
              target: 'active',
            },
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ event }) => (event.output.ok ? event.output.value : null),
              }),
              target: 'resolved',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                recovery: ({ event }) =>
                  !event.output.ok && event.output.error.retryable
                    ? ('reconcile' as const)
                    : ('none' as const),
              }),
              target: 'failed',
            },
          ],
          onError: {
            actions: assign({
              failure: { ...unexpectedFailure, retryable: true },
              recovery: 'reconcile' as const,
            }),
            target: 'failed',
          },
        },
      },
      revoking: {
        invoke: {
          id: 'revoke-handoff',
          src: 'revoke',
          input: ({ context }) => ({ handoffId: context.request?.handoffId ?? null }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok,
              actions: assign({
                failure: null,
                recovery: 'none' as const,
                snapshot: ({ event }) => (event.output.ok ? event.output.value : null),
              }),
              target: 'resolved',
            },
            {
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
                recovery: ({ event }) =>
                  !event.output.ok && event.output.error.retryable
                    ? ('revoke' as const)
                    : ('none' as const),
              }),
              target: 'failed',
            },
          ],
          onError: {
            actions: assign({
              failure: { ...unexpectedFailure, retryable: true },
              recovery: 'revoke' as const,
            }),
            target: 'failed',
          },
        },
      },
      failed: {
        on: {
          RETRY: [
            { guard: 'canRetryTargets', target: 'resolvingTargets' },
            { guard: 'canRetryApprove', target: 'submitting' },
            { guard: 'canRetryReconcile', target: 'reconciling' },
            { guard: 'canRetryRevoke', target: 'revoking' },
          ],
        },
      },
      resolved: { type: 'final' },
    },
  });
}

export type HandoffDeliveryMachine = ReturnType<typeof createHandoffDeliveryMachine>;

function snapshotFromSubmission(
  request: HandoffApprovalRequest | null,
  outcome: HandoffSubmissionOutcome,
): HandoffSnapshot | null {
  if (outcome.kind === 'resolved') {
    return outcome.snapshot;
  }
  return request === null
    ? null
    : {
        expiresAtUnixMs: request.expiresAtUnixMs,
        handoffId: outcome.handoffId,
        status: 'approved',
      };
}
