import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import type { TranslationKey } from '@/shared/i18n/resources';

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
