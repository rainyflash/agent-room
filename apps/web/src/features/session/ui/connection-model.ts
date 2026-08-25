import type {
  AuthenticationTarget,
  SessionContext,
} from '@/features/session/domain/session-machine';
import type { StatusTone } from '@agent-room/ui-system';
import type { TranslationKey } from '@/shared/i18n/resources';

export type SessionStateName =
  | 'authenticating'
  | 'booting'
  | 'degraded'
  | 'offline'
  | 'ready'
  | 'reconnecting'
  | 'restoring'
  | 'signingOut'
  | 'syncing'
  | 'unauthenticated';

export type ConnectionAction = 'enter' | 'login' | 'logout' | 'retry';
export type StageStatus = 'blocked' | 'complete' | 'current' | 'pending';

export type ConnectionStage = {
  readonly detailKey: TranslationKey;
  readonly index: number;
  readonly status: StageStatus;
  readonly titleKey: TranslationKey;
};

export type ConnectionViewModel = {
  readonly action: ConnectionAction | null;
  readonly actionKey: TranslationKey | null;
  readonly busy: boolean;
  readonly currentStage: number;
  readonly detailKey: TranslationKey;
  readonly failureKey: TranslationKey | null;
  readonly stages: readonly ConnectionStage[];
  readonly state: SessionStateName;
  readonly statusKey: TranslationKey;
  readonly titleKey: TranslationKey;
  readonly tone: StatusTone;
};

const states = new Set<SessionStateName>([
  'authenticating',
  'booting',
  'degraded',
  'offline',
  'ready',
  'reconnecting',
  'restoring',
  'signingOut',
  'syncing',
  'unauthenticated',
]);

const stageDefinitions = [
  ['connection.stage.runtime.title', 'connection.stage.runtime.detail'],
  ['connection.stage.control.title', 'connection.stage.control.detail'],
  ['connection.stage.matrix.title', 'connection.stage.matrix.detail'],
  ['connection.stage.sync.title', 'connection.stage.sync.detail'],
  ['connection.stage.ready.title', 'connection.stage.ready.detail'],
] as const;

const stateStage: Readonly<Record<Exclude<SessionStateName, 'degraded' | 'offline'>, number>> = {
  authenticating: 1,
  booting: 0,
  ready: 4,
  reconnecting: 3,
  restoring: 2,
  signingOut: 1,
  syncing: 3,
  unauthenticated: 1,
};

const stateCopy: Readonly<
  Record<Exclude<SessionStateName, 'unauthenticated'>, readonly [TranslationKey, TranslationKey]>
> = {
  authenticating: [
    'connection.state.authenticating.title',
    'connection.state.authenticating.detail',
  ],
  booting: ['connection.state.booting.title', 'connection.state.booting.detail'],
  degraded: ['connection.state.degraded.title', 'connection.state.degraded.detail'],
  offline: ['connection.state.offline.title', 'connection.state.offline.detail'],
  ready: ['connection.state.ready.title', 'connection.state.ready.detail'],
  reconnecting: ['connection.state.reconnecting.title', 'connection.state.reconnecting.detail'],
  restoring: ['connection.state.restoring.title', 'connection.state.restoring.detail'],
  signingOut: ['connection.state.signingOut.title', 'connection.state.signingOut.detail'],
  syncing: ['connection.state.syncing.title', 'connection.state.syncing.detail'],
};

const failureCopy: Readonly<Record<string, TranslationKey>> = {
  'matrix.crypto_initialization_failed': 'connection.state.failure.matrixCrypto',
  'matrix.identity_mismatch': 'connection.state.failure.identityMismatch',
  'matrix.initial_sync_failed': 'connection.state.failure.matrixSync',
  'matrix.initial_sync_timeout': 'connection.state.failure.matrixSync',
  'matrix.invalid_login_response': 'connection.state.failure.matrixLogin',
  'matrix.invalid_sso_callback_path': 'connection.state.failure.matrixLogin',
  'matrix.login_exchange_failed': 'connection.state.failure.matrixLogin',
  'matrix.sso_start_failed': 'connection.state.failure.matrixLogin',
  'matrix.sso_unavailable': 'connection.state.failure.matrixLogin',
};

const busyStates = new Set<SessionStateName>([
  'authenticating',
  'booting',
  'reconnecting',
  'restoring',
  'signingOut',
  'syncing',
]);

const statusByState: Readonly<Record<SessionStateName, readonly [TranslationKey, StatusTone]>> = {
  authenticating: ['connection.status.connecting', 'network'],
  booting: ['connection.status.connecting', 'network'],
  degraded: ['connection.status.degraded', 'alert'],
  offline: ['connection.status.offline', 'offline'],
  ready: ['connection.status.live', 'active'],
  reconnecting: ['connection.status.connecting', 'network'],
  restoring: ['connection.status.connecting', 'network'],
  signingOut: ['connection.status.connecting', 'network'],
  syncing: ['connection.status.connecting', 'network'],
  unauthenticated: ['connection.status.actionRequired', 'network'],
};

export function sessionStateName(value: unknown): SessionStateName {
  return typeof value === 'string' && states.has(value as SessionStateName)
    ? (value as SessionStateName)
    : 'degraded';
}

export function connectionViewModel(
  state: SessionStateName,
  context: SessionContext,
): ConnectionViewModel {
  const [titleKey, detailKey] = copyForState(state, context.authenticationTarget);
  const currentStage = stageForState(state, context);
  const blocked = state === 'degraded' || state === 'offline';
  const [action, actionKey] = actionForState(state, context);
  const [statusKey, tone] = statusByState[state];

  return {
    action,
    actionKey,
    busy: busyStates.has(state),
    currentStage,
    detailKey,
    failureKey: failureMessage(context),
    stages: stageDefinitions.map(([stageTitleKey, stageDetailKey], index) => ({
      detailKey: stageDetailKey,
      index,
      status:
        state === 'ready' || index < currentStage
          ? 'complete'
          : index === currentStage
            ? blocked
              ? 'blocked'
              : 'current'
            : 'pending',
      titleKey: stageTitleKey,
    })),
    state,
    statusKey,
    titleKey,
    tone,
  };
}

function copyForState(
  state: SessionStateName,
  target: AuthenticationTarget,
): readonly [TranslationKey, TranslationKey] {
  if (state !== 'unauthenticated') {
    return stateCopy[state];
  }
  return target === 'control'
    ? [
        'connection.state.unauthenticated.control.title',
        'connection.state.unauthenticated.control.detail',
      ]
    : [
        'connection.state.unauthenticated.matrix.title',
        'connection.state.unauthenticated.matrix.detail',
      ];
}

function stageForState(state: SessionStateName, context: SessionContext): number {
  if (state !== 'degraded' && state !== 'offline') {
    if (state === 'authenticating' || state === 'unauthenticated') {
      return context.authenticationTarget === 'control' ? 1 : 2;
    }
    return stateStage[state];
  }
  if (context.principal === null) {
    return 1;
  }
  return context.connection === null ? 2 : 3;
}

function actionForState(
  state: SessionStateName,
  context: SessionContext,
): readonly [ConnectionAction | null, TranslationKey | null] {
  const strategies: Readonly<
    Partial<Record<SessionStateName, readonly [ConnectionAction, TranslationKey]>>
  > = {
    offline: ['retry', 'connection.action.retry'],
    ready: ['enter', 'connection.action.enter'],
    reconnecting: ['retry', 'connection.action.retry'],
    unauthenticated: [
      'login',
      context.authenticationTarget === 'control'
        ? 'connection.action.loginControl'
        : 'connection.action.loginMatrix',
    ],
  };
  if (state === 'degraded') {
    return context.failure?.retryable === true
      ? ['retry', 'connection.action.retry']
      : context.principal === null
        ? ['login', 'connection.action.loginControl']
        : ['logout', 'connection.action.logout'];
  }
  return strategies[state] ?? [null, null];
}

function failureMessage(context: SessionContext): TranslationKey | null {
  const failure = context.failure;
  if (failure === null) {
    return null;
  }
  const exact = failureCopy[failure.code];
  if (exact !== undefined) {
    return exact;
  }
  const boundaryCopy: Readonly<Record<typeof failure.boundary, TranslationKey>> = {
    browser: 'connection.state.failure.generic',
    'control-plane': 'connection.state.failure.control',
    identity: 'connection.state.failure.identityMismatch',
    matrix: 'connection.state.failure.generic',
  };
  return failure.offline ? 'connection.state.failure.offline' : boundaryCopy[failure.boundary];
}
