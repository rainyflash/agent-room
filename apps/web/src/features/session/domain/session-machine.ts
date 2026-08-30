import { assign, fromPromise, setup } from 'xstate';

import type {
  AuthenticationStartOutcome,
  MatrixConnection,
  SessionDependencies,
  SessionFailure,
  WebSession,
} from './session';
import { err, ok, type Result } from '@/shared/result';

export type AuthenticationTarget = 'control' | 'matrix';

export type SessionContext = {
  readonly authenticationTarget: AuthenticationTarget;
  readonly connection: MatrixConnection | null;
  readonly failure: SessionFailure | null;
  readonly principal: WebSession | null;
  readonly resumePath: string | null;
};

export type SessionEvent =
  | { readonly type: 'CONTROL_DEGRADED'; readonly failure: SessionFailure }
  | { readonly type: 'CONTROL_HEALTHY' }
  | { readonly type: 'LOGIN' }
  | { readonly type: 'LOGOUT' }
  | { readonly type: 'MATRIX_INTERRUPTED' }
  | { readonly type: 'MATRIX_RESTORED' }
  | { readonly type: 'OFFLINE' }
  | { readonly type: 'ONLINE' }
  | { readonly type: 'RETRY' };

type ControlSessionOutcome =
  | { readonly kind: 'authenticated'; readonly session: WebSession }
  | { readonly kind: 'failure'; readonly failure: SessionFailure }
  | { readonly kind: 'unauthenticated' };

const unexpectedFailure: SessionFailure = {
  boundary: 'browser',
  code: 'session.unexpected_failure',
  offline: false,
  retryable: true,
};

export function createSessionMachine(dependencies: SessionDependencies) {
  const loadControlSession = fromPromise(async (): Promise<ControlSessionOutcome> => {
    const result = await dependencies.controlPlane.readSession();
    if (result.ok) {
      return { kind: 'authenticated', session: result.value };
    }
    return result.error.code === 'authentication.session_required'
      ? { kind: 'unauthenticated' }
      : { failure: result.error, kind: 'failure' };
  });

  const restoreMatrix = fromPromise<
    Awaited<ReturnType<SessionDependencies['matrix']['restore']>>,
    { readonly principal: WebSession | null }
  >(async ({ input }) => {
    const principal = input.principal;
    return principal === null
      ? err(unexpectedFailure)
      : await dependencies.matrix.restore(principal.matrixUserId);
  });

  const authenticate = fromPromise<
    Result<AuthenticationStartOutcome, SessionFailure>,
    { readonly returnPath: string; readonly target: AuthenticationTarget }
  >(async ({ input }) => {
    if (input.target === 'control') {
      return await dependencies.controlPlane.beginAuthentication(input.returnPath);
    }
    const matrix = await dependencies.matrix.beginAuthentication(input.returnPath);
    return matrix.ok ? ok({ kind: 'browser-navigation' }) : matrix;
  });

  const synchronize = fromPromise<
    Result<void, SessionFailure>,
    { readonly connection: MatrixConnection | null }
  >(async ({ input }) => {
    const connection = input.connection;
    return connection === null ? err(unexpectedFailure) : await connection.waitUntilPrepared();
  });

  const signOut = fromPromise(async (): Promise<Result<void, SessionFailure>> => {
    const matrixResult = await dependencies.matrix.logout();
    const controlResult = await dependencies.controlPlane.logout();
    return !matrixResult.ok ? matrixResult : controlResult;
  });

  return setup({
    types: {
      context: {} as SessionContext,
      events: {} as SessionEvent,
    },
    actors: {
      authenticate,
      loadControlSession,
      restoreMatrix,
      signOut,
      synchronize,
    },
    actions: {
      clearFailure: assign({ failure: null }),
      clearResumePath: assign({ resumePath: null }),
      clearSession: assign({
        authenticationTarget: 'control',
        connection: null,
        failure: null,
        principal: null,
        resumePath: null,
      }),
      resumeRequestedRoute: ({ context }) => {
        if (context.resumePath !== null && context.resumePath !== '/connect') {
          dependencies.browser.replacePath(context.resumePath);
        }
      },
      setControlFailure: assign({
        failure: ({ event }) =>
          event.type === 'CONTROL_DEGRADED' ? event.failure : unexpectedFailure,
      }),
      setNavigationFailure: assign({
        failure: {
          boundary: 'browser',
          code: 'browser.authentication_navigation_stalled',
          offline: false,
          retryable: true,
        },
      }),
      setUnexpectedFailure: assign({ failure: unexpectedFailure }),
    },
  }).createMachine({
    id: 'web-session',
    initial: dependencies.browser.isOnline() ? 'booting' : 'offline',
    context: {
      authenticationTarget: 'control',
      connection: null,
      failure: null,
      principal: null,
      resumePath: null,
    },
    states: {
      booting: {
        invoke: {
          id: 'load-control-session',
          src: 'loadControlSession',
          onDone: [
            {
              guard: ({ event }) => event.output.kind === 'authenticated',
              target: 'restoring',
              actions: assign({
                failure: null,
                principal: ({ event }) => {
                  return event.output.kind === 'authenticated' ? event.output.session : null;
                },
              }),
            },
            {
              guard: ({ event }) => event.output.kind === 'unauthenticated',
              target: 'unauthenticated',
              actions: assign({
                authenticationTarget: () => {
                  return 'control' as const;
                },
                failure: null,
              }),
            },
            {
              guard: ({ event }) => event.output.kind === 'failure' && event.output.failure.offline,
              target: 'offline',
              actions: assign({
                failure: ({ event }) =>
                  event.output.kind === 'failure' ? event.output.failure : unexpectedFailure,
              }),
            },
            {
              target: 'degraded',
              actions: assign({
                failure: ({ event }) =>
                  event.output.kind === 'failure' ? event.output.failure : unexpectedFailure,
              }),
            },
          ],
          onError: {
            target: 'degraded',
            actions: 'setUnexpectedFailure',
          },
        },
        on: { OFFLINE: 'offline' },
      },
      unauthenticated: {
        on: {
          LOGIN: {
            target: 'authenticating',
          },
          OFFLINE: 'offline',
          RETRY: 'booting',
        },
      },
      authenticating: {
        invoke: {
          id: 'authenticate-session',
          src: 'authenticate',
          input: ({ context }) => ({
            returnPath: dependencies.browser.currentPath(),
            target: context.authenticationTarget,
          }),
          onDone: [
            {
              guard: ({ event }) =>
                event.output.ok && event.output.value.kind === 'session-established',
              target: 'booting',
              actions: 'clearFailure',
            },
            {
              guard: ({ event }) =>
                event.output.ok && event.output.value.kind === 'browser-navigation',
              target: 'awaitingBrowserNavigation',
            },
            {
              guard: ({ event }) => !event.output.ok && event.output.error.offline,
              target: 'offline',
              actions: assign({
                failure: ({ event }) => (event.output.ok ? null : event.output.error),
              }),
            },
            {
              guard: ({ event }) => !event.output.ok,
              target: 'degraded',
              actions: assign({
                failure: ({ event }) => (event.output.ok ? null : event.output.error),
              }),
            },
          ],
          onError: {
            target: 'degraded',
            actions: 'setUnexpectedFailure',
          },
        },
        on: { OFFLINE: 'offline', RETRY: 'booting' },
      },
      awaitingBrowserNavigation: {
        after: {
          10_000: {
            actions: 'setNavigationFailure',
            target: 'degraded',
          },
        },
        on: { OFFLINE: 'offline', RETRY: 'booting' },
      },
      restoring: {
        invoke: {
          id: 'restore-matrix-session',
          src: 'restoreMatrix',
          input: ({ context }) => ({ principal: context.principal }),
          onDone: [
            {
              guard: ({ event }) => event.output.ok && event.output.value.kind === 'connected',
              target: 'syncing',
              actions: assign({
                connection: ({ event }) => {
                  return event.output.ok && event.output.value.kind === 'connected'
                    ? event.output.value.connection
                    : null;
                },
                failure: null,
                resumePath: ({ event }) => {
                  return event.output.ok && event.output.value.kind === 'connected'
                    ? (event.output.value.returnPath ?? null)
                    : null;
                },
              }),
            },
            {
              guard: ({ event }) =>
                event.output.ok && event.output.value.kind === 'authentication-required',
              target: 'unauthenticated',
              actions: assign({
                authenticationTarget: () => {
                  return 'matrix' as const;
                },
                failure: null,
              }),
            },
            {
              guard: ({ event }) => !event.output.ok && event.output.error.offline,
              target: 'offline',
              actions: assign({
                failure: ({ event }) => (event.output.ok ? null : event.output.error),
              }),
            },
            {
              target: 'degraded',
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
              }),
            },
          ],
          onError: {
            target: 'degraded',
            actions: 'setUnexpectedFailure',
          },
        },
        on: { OFFLINE: 'offline' },
      },
      syncing: {
        invoke: {
          id: 'synchronize-matrix',
          src: 'synchronize',
          input: ({ context }) => ({ connection: context.connection }),
          onDone: [
            { guard: ({ event }) => event.output.ok, target: 'ready' },
            {
              guard: ({ event }) => !event.output.ok && event.output.error.offline,
              target: 'offline',
              actions: assign({
                failure: ({ event }) => (event.output.ok ? null : event.output.error),
              }),
            },
            {
              target: 'degraded',
              actions: assign({
                failure: ({ event }) => (event.output.ok ? unexpectedFailure : event.output.error),
              }),
            },
          ],
          onError: {
            target: 'degraded',
            actions: 'setUnexpectedFailure',
          },
        },
        on: { OFFLINE: 'offline' },
      },
      ready: {
        entry: ['clearFailure', 'resumeRequestedRoute', 'clearResumePath'],
        on: {
          CONTROL_DEGRADED: { target: 'degraded', actions: 'setControlFailure' },
          LOGOUT: 'signingOut',
          MATRIX_INTERRUPTED: 'reconnecting',
          OFFLINE: 'offline',
          RETRY: 'restoring',
        },
      },
      degraded: {
        on: {
          CONTROL_HEALTHY: [
            {
              guard: ({ context }) =>
                context.failure?.boundary === 'control-plane' && context.connection !== null,
              target: 'ready',
            },
            {
              guard: ({ context }) => context.failure?.boundary === 'control-plane',
              target: 'booting',
            },
          ],
          LOGIN: {
            target: 'authenticating',
          },
          LOGOUT: 'signingOut',
          MATRIX_INTERRUPTED: 'reconnecting',
          OFFLINE: 'offline',
          RETRY: 'booting',
        },
      },
      reconnecting: {
        after: { 1_200: 'restoring' },
        on: {
          MATRIX_RESTORED: 'ready',
          OFFLINE: 'offline',
          RETRY: 'restoring',
        },
      },
      offline: {
        on: {
          LOGOUT: 'signingOut',
          ONLINE: 'booting',
          RETRY: 'booting',
        },
      },
      signingOut: {
        invoke: {
          id: 'sign-out',
          src: 'signOut',
          onDone: { target: 'unauthenticated', actions: 'clearSession' },
          onError: { target: 'unauthenticated', actions: 'clearSession' },
        },
      },
    },
  });
}

export type SessionMachine = ReturnType<typeof createSessionMachine>;
