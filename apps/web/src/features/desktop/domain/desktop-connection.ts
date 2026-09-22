import type { BridgePhase, BridgeRuntime } from '@/features/desktop/domain/desktop-runtime';
import type { TranslationKey } from '@/shared/i18n/resources';

type BridgeLifecycle = BridgeRuntime['lifecycle'];

export const desktopPhaseMessage: Readonly<Record<BridgePhase, TranslationKey>> = {
  authorization_required: 'desktop.phase.authorizationRequired',
  authorized: 'desktop.phase.authorized',
  discovering: 'desktop.phase.discovering',
  halted: 'desktop.phase.halted',
  ready: 'desktop.phase.ready',
  retry_scheduled: 'desktop.phase.retryScheduled',
  starting: 'desktop.phase.starting',
  reconnecting: 'desktop.phase.reconnecting',
  stopped: 'desktop.phase.stopped',
};

// Bridge 进程还在，只是连不上服务器，按自己的退避等下一次尝试；与进程崩溃后的自动重启区分开。
export function waitingForServer(lifecycle: BridgeLifecycle | undefined): boolean {
  return (
    lifecycle?.phase === 'retry_scheduled' &&
    lifecycle.diagnosticCode === 'desktop.bridge.server_unreachable'
  );
}

export function desktopPhaseLabel(lifecycle: BridgeLifecycle | undefined): TranslationKey {
  return waitingForServer(lifecycle)
    ? 'desktop.phase.serverUnreachable'
    : desktopPhaseMessage[lifecycle?.phase ?? 'discovering'];
}

export function localConnectionReady(phase: BridgePhase): boolean {
  return phase === 'authorized' || phase === 'ready';
}

export const localConnectionNotice: Readonly<
  Record<Exclude<BridgePhase, 'ready' | 'authorized'>, TranslationKey>
> = {
  discovering: 'agentInvite.runtime.starting',
  starting: 'agentInvite.runtime.starting',
  reconnecting: 'agentInvite.runtime.reconnecting',
  authorization_required: 'agentInvite.runtime.authorize',
  retry_scheduled: 'agentInvite.runtime.retrying',
  halted: 'agentInvite.runtime.stopped',
  stopped: 'agentInvite.runtime.stopped',
};

// 设备码过期只需重试获取新代码，不应与其他授权失败共用笼统说明。
export function authorizationFailureMessage(
  lastFailureCode: string | null | undefined,
): TranslationKey {
  return lastFailureCode === 'bridge.authorization_expired'
    ? 'desktop.authorization.expiredDescription'
    : 'desktop.authorization.failedDescription';
}

// The Bridge names why it went offline; say what a person can do about the common ones.
export function haltReasonMessage(code: string | null | undefined): TranslationKey {
  switch (code) {
    case 'bridge.secure_storage_unavailable':
    case 'bridge.secure_storage_corrupt':
      return 'desktop.halted.reason.secureStorage';
    case 'bridge.control_plane_response_invalid':
    case 'bridge.matrix_response_invalid':
      return 'desktop.halted.reason.responseInvalid';
    case 'bridge.matrix_crypto_identity_conflict':
      return 'desktop.halted.reason.identityConflict';
    case 'bridge.matrix_room_not_found':
      return 'desktop.halted.reason.roomMissing';
    default:
      return 'desktop.halted.description';
  }
}

export function hostFailureMessage(code: string): TranslationKey {
  switch (code) {
    case 'codex.config_incompatible':
      return 'agentInvite.host.incompatible';
    case 'codex.config_invalid':
      return 'agentInvite.host.invalidConfig';
    case 'codex.executable_invalid':
      return 'agentInvite.host.invalidExecutable';
    case 'host.command_timed_out':
      return 'agentInvite.host.timedOut';
    case 'host.concurrent_modification':
      return 'agentInvite.host.concurrentChange';
    case 'codex.list_failed':
    case 'codex.list_invalid':
      return 'agentInvite.host.readFailed';
    default:
      return 'agentInvite.host.failed';
  }
}
