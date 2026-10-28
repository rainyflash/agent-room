import type { BridgePhase } from './desktop-runtime';
import { localConnectionReady } from './desktop-connection';

/** 桌面端开着时多久报一次：这段时间里本机连接连着的时间占多少。 */
export const BRIDGE_UPTIME_WINDOW_MS = 15 * 60 * 1_000;

// 本机连接该连着的时候才算：还没授权、停下了、刚打开还在找 Bridge，都不算没连上。
function countsTowardUptime(phase: BridgePhase): boolean {
  return phase !== 'discovering' && phase !== 'authorization_required' && phase !== 'stopped';
}

/**
 * 按时间算本机连接连着的比例。以前每次状态变了报一个 0 或 1，每次启动都先报一个“正在启动”，
 * 比例到不了一半，告警一直响（agent-room-ops#39）。时间用单调的钟（毫秒），由调用的一方给。
 */
export class BridgeUptimeWindow {
  #phase: BridgePhase | null = null;
  #since = 0;
  #ready = 0;
  #counted = 0;

  observe(phase: BridgePhase, now: number): void {
    this.#settle(now);
    this.#phase = phase;
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
    if (this.#phase === null || !countsTowardUptime(this.#phase)) return;
    this.#counted += elapsed;
    if (localConnectionReady(this.#phase)) this.#ready += elapsed;
  }
}
