import type { StatusTone } from '@agent-room/ui-system';

import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import type { ConnectionStage } from '@/features/session/ui/connection-model';
import type { TranslationKey } from '@/shared/i18n/resources';

const stageDefinitions = [
  ['desktop.connection.stage.bridge.title', 'desktop.connection.stage.bridge.detail'],
  ['desktop.connection.stage.authorization.title', 'desktop.connection.stage.authorization.detail'],
  ['desktop.connection.stage.agent.title', 'desktop.connection.stage.agent.detail'],
  ['desktop.connection.stage.lobby.title', 'desktop.connection.stage.lobby.detail'],
] as const;

const currentStageByPhase: Readonly<Record<BridgePhase, number>> = {
  authorization_required: 1,
  authorized: 2,
  discovering: 0,
  halted: 0,
  ready: 3,
  retry_scheduled: 0,
  starting: 0,
  stopped: 0,
};

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

const copyByPhase: Readonly<Record<BridgePhase, readonly [TranslationKey, TranslationKey]>> = {
  authorization_required: [
    'desktop.connection.state.authorization.title',
    'desktop.connection.state.authorization.detail',
  ],
  authorized: ['desktop.connection.state.agent.title', 'desktop.connection.state.agent.detail'],
  discovering: [
    'desktop.connection.state.starting.title',
    'desktop.connection.state.starting.detail',
  ],
  halted: ['desktop.connection.state.halted.title', 'desktop.connection.state.halted.detail'],
  ready: ['desktop.connection.state.ready.title', 'desktop.connection.state.ready.detail'],
  retry_scheduled: [
    'desktop.connection.state.starting.title',
    'desktop.connection.state.starting.detail',
  ],
  starting: ['desktop.connection.state.starting.title', 'desktop.connection.state.starting.detail'],
  stopped: ['desktop.connection.state.stopped.title', 'desktop.connection.state.stopped.detail'],
};

const toneByPhase: Readonly<Record<BridgePhase, StatusTone>> = {
  authorization_required: 'network',
  authorized: 'network',
  discovering: 'idle',
  halted: 'alert',
  ready: 'active',
  retry_scheduled: 'network',
  starting: 'network',
  stopped: 'offline',
};

export type DesktopConnectionView = {
  readonly busy: boolean;
  readonly currentStage: number;
  readonly detailKey: TranslationKey;
  readonly stages: readonly ConnectionStage[];
  readonly statusKey: TranslationKey;
  readonly titleKey: TranslationKey;
  readonly tone: StatusTone;
};

export function desktopConnectionView(
  phase: BridgePhase,
  commandBusy: boolean,
  failed: boolean,
): DesktopConnectionView {
  const currentStage = currentStageByPhase[phase];
  const blocked = failed || phase === 'halted' || phase === 'stopped';
  const [titleKey, detailKey] = copyByPhase[phase];
  return {
    busy:
      commandBusy ||
      phase === 'discovering' ||
      phase === 'starting' ||
      phase === 'retry_scheduled' ||
      phase === 'authorized',
    currentStage,
    detailKey,
    stages: stageDefinitions.map(([stageTitleKey, stageDetailKey], index) => ({
      detailKey: stageDetailKey,
      index,
      status:
        phase === 'ready' || index < currentStage
          ? 'complete'
          : index === currentStage
            ? blocked
              ? 'blocked'
              : 'current'
            : 'pending',
      titleKey: stageTitleKey,
    })),
    statusKey: desktopPhaseMessage[phase],
    titleKey,
    tone: failed ? 'alert' : toneByPhase[phase],
  };
}
