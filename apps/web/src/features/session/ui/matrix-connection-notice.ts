import type { SessionContext } from '@/features/session/domain/session-machine';
import type { TranslationKey } from '@/shared/i18n/resources';
import { connectionViewModel, type SessionStateName } from '@/features/session/ui/connection-model';

/**
 * 账户登录着、这台设备的消息却没连上时，房间页和提示栈怎么说。
 *
 * - `connecting`：正在连，等一会就好；
 * - `actionRequired`：要人动手。`action` 是连接页上的那个动作（重新连接或登录）；
 *   连接页给的是“退出登录”这类重的动作时为 `null`，界面只给“查看详情”，去连接页再选。
 */
export type MatrixConnectionNotice =
  | { readonly kind: 'connecting' }
  | {
      readonly kind: 'actionRequired';
      readonly action: 'login' | 'retry' | null;
      readonly actionKey: TranslationKey | null;
      readonly failureKey: TranslationKey | null;
    };

const connectingStates = new Set<SessionStateName>([
  'booting',
  'invalidating',
  'reconnecting',
  'restoring',
  'syncing',
]);

const actionRequiredStates = new Set<SessionStateName>(['degraded', 'offline', 'unauthenticated']);

/**
 * 没登录账户（连接页自己会说）、消息已经连上、或者正在退出登录时返回 `null`。
 * 登录 Matrix 时算“正在连”：网页端是整页跳去登录页，桌面端由控制面直接发，都不用人过去。
 */
export function matrixConnectionNotice(
  state: SessionStateName,
  context: SessionContext,
): MatrixConnectionNotice | null {
  if (context.principal === null || state === 'ready') {
    return null;
  }
  if (state === 'authenticating' || state === 'awaitingBrowserNavigation') {
    return context.authenticationTarget === 'matrix' ? { kind: 'connecting' } : null;
  }
  if (connectingStates.has(state)) {
    return { kind: 'connecting' };
  }
  if (!actionRequiredStates.has(state)) {
    return null;
  }
  // 控制面出错而消息还连着时，房间照常能用，不算消息没连上。
  if (context.connection !== null && context.failure?.boundary !== 'matrix') {
    return null;
  }
  const view = connectionViewModel(state, context);
  const action = view.action === 'login' || view.action === 'retry' ? view.action : null;
  return {
    kind: 'actionRequired',
    action,
    actionKey: action === null ? null : view.actionKey,
    failureKey: view.failureKey,
  };
}
