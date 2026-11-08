import type { BridgePhase, BridgeRuntime } from './desktop-runtime';
import { localConnectionReady } from './desktop-connection';

/** 桌面端开着时多久报一次：这段时间里本机连接连着的时间占多少。 */
export const BRIDGE_UPTIME_WINDOW_MS = 15 * 60 * 1_000;

/** 托管的 Bridge 在等电脑授权、应用里还没登录时，桌面端在“正在启动”上带的说明。 */
const WAITING_FOR_SIGN_IN_DIAGNOSTIC = 'desktop.bridge.waiting_for_sign_in';

/** 算比例时看的状态：本机连接的阶段，或者新装的电脑正在等人登录（阶段照旧是“正在启动”）。 */
export type BridgeUptimeState = BridgePhase | 'waiting_for_sign_in';

export function bridgeUptimeState(
  lifecycle: Pick<BridgeRuntime['lifecycle'], 'diagnosticCode' | 'phase'> | undefined,
): BridgeUptimeState {
  if (lifecycle === undefined) return 'discovering';
  return lifecycle.phase === 'starting' &&
    lifecycle.diagnosticCode === WAITING_FOR_SIGN_IN_DIAGNOSTIC
    ? 'waiting_for_sign_in'
    : lifecycle.phase;
}

export function bridgeUptimeReady(state: BridgeUptimeState): boolean {
  return state !== 'waiting_for_sign_in' && localConnectionReady(state);
}

// 本机连接该连着的时候才算：还没授权、停下了、刚打开还在找 Bridge、新装的电脑在等人登录，都不算没连上。
function countsTowardUptime(state: BridgeUptimeState): boolean {
  return (
    state !== 'discovering' &&
    state !== 'authorization_required' &&
    state !== 'stopped' &&
    state !== 'waiting_for_sign_in'
  );
}

/**
 * 按时间算本机连接连着的比例。以前每次状态变了报一个 0 或 1，每次启动都先报一个“正在启动”，
 * 比例到不了一半，告警一直响（agent-room-ops#39）。时间用单调的钟（毫秒），由调用的一方给。
 */
export class BridgeUptimeWindow {
  #state: BridgeUptimeState | null = null;
  #since = 0;
  #ready = 0;
  #counted = 0;

  observe(state: BridgeUptimeState, now: number): void {
    this.#settle(now);
    this.#state = state;
  }

  /** 从上次取到现在连着的比例，然后从头算；这段时间本机连接都不该连着时给 `null`。 */
  take(now: number): number | null {
    this.#settle(now);
    const share = this.#counted > 0 ? this.#ready / this.#counted : null;
    this.#ready = 0;
    this.#counted = 0;
    return share;
  }

  #settle(now: number): void {
    const elapsed = Math.max(0, now - this.#since);
    this.#since = now;
    if (this.#state === null || !countsTowardUptime(this.#state)) return;
    this.#counted += elapsed;
    if (bridgeUptimeReady(this.#state)) this.#ready += elapsed;
  }
}
