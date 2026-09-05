export type SceneRenderFrame = {
  readonly elapsedSeconds: number;
  readonly animated: boolean;
  readonly invalidated: boolean;
};

type FrameHost = {
  readonly request: (callback: (now: number) => void) => number;
  readonly cancel: (id: number) => void;
  readonly now: () => number;
  readonly render: (frame: SceneRenderFrame) => void;
};

/** 动画与交互共用一条帧队列，同一显示帧最多提交一次绘制。 */
export class SceneFrameScheduler {
  readonly #host: FrameHost;
  #frame: number | null = null;
  #animating = false;
  #invalidated = false;
  #destroyed = false;
  #elapsedSeconds = 0;
  #lastAnimationAt = 0;

  constructor(host: FrameHost) {
    this.#host = host;
  }

  setAnimating(active: boolean): void {
    if (this.#destroyed) return;
    this.#animating = active;
    this.#lastAnimationAt = this.#host.now();
    this.invalidate();
  }

  invalidate(): void {
    if (this.#destroyed) return;
    this.#invalidated = true;
    this.#requestFrame();
  }

  destroy(): void {
    this.#destroyed = true;
    if (this.#frame !== null) this.#host.cancel(this.#frame);
    this.#frame = null;
  }

  readonly #tick = (now: number): void => {
    this.#frame = null;
    if (this.#destroyed) return;
    const elapsed = now - this.#lastAnimationAt;
    const animated = this.#animating && elapsed >= 1000 / 30;
    if (animated) {
      this.#elapsedSeconds += Math.min(elapsed, 100) / 1000;
      this.#lastAnimationAt = now;
    }
    if (this.#invalidated || animated) {
      const invalidated = this.#invalidated;
      this.#invalidated = false;
      this.#host.render({ elapsedSeconds: this.#elapsedSeconds, animated, invalidated });
    }
    if (this.#animating) this.#requestFrame();
  };

  #requestFrame(): void {
    if (this.#frame === null && !this.#destroyed) this.#frame = this.#host.request(this.#tick);
  }
}
