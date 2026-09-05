type DepthTarget = { zIndex: number };

/** 真实深度决定遮挡；只有相对顺序改变才使渲染批次失效。 */
export class SceneDepthOrder {
  readonly #depths = new Map<DepthTarget, number>();

  set(target: DepthTarget, depth: number): void {
    this.#depths.set(target, depth);
  }

  delete(target: DepthTarget): void {
    this.#depths.delete(target);
  }

  clear(): void {
    this.#depths.clear();
  }

  apply(): void {
    const ordered = [...this.#depths].sort((left, right) => left[1] - right[1]);
    for (const [rank, [target]] of ordered.entries()) {
      if (target.zIndex !== rank) target.zIndex = rank;
    }
  }
}
