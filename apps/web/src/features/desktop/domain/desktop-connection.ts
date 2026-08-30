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
  stopped: 'desktop.phase.stopped',
};
