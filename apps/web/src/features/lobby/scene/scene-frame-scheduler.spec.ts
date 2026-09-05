import { describe, expect, it } from 'vitest';
import { SceneFrameScheduler, type SceneRenderFrame } from './scene-frame-scheduler';

function createHarness(onRender?: () => void) {
  let now = 0;
  let sequence = 0;
  const pending = new Map<number, (time: number) => void>();
  const rendered: SceneRenderFrame[] = [];
  const scheduler = new SceneFrameScheduler({
    request: (callback) => {
      const id = ++sequence;
      pending.set(id, callback);
      return id;
    },
    cancel: (id) => pending.delete(id),
    now: () => now,
    render: (frame) => {
      rendered.push(frame);
      onRender?.();
    },
  });
  return {
    scheduler,
    pending,
    rendered,
    tick: (time: number) => {
      now = time;
      const callbacks = [...pending.values()];
      pending.clear();
      for (const callback of callbacks) callback(now);
    },
  };
}

describe('SceneFrameScheduler', () => {
  it('合并同帧的多次交互与到期动画，保留两种遥测语义', () => {
    const harness = createHarness();
    harness.scheduler.setAnimating(true);
    harness.tick(0);
    harness.scheduler.invalidate();
    harness.scheduler.invalidate();
    expect(harness.pending.size).toBe(1);
    harness.tick(40);
    expect(harness.rendered).toEqual([
      { elapsedSeconds: 0, animated: false, invalidated: true },
      { elapsedSeconds: 0.04, animated: true, invalidated: true },
    ]);
    expect(harness.pending.size).toBe(1);
  });

  it('限制动画频率，但不延迟交互到下一次动画', () => {
    const harness = createHarness();
    harness.scheduler.setAnimating(true);
    harness.tick(0);
    harness.tick(16);
    expect(harness.rendered).toHaveLength(1);
    harness.scheduler.invalidate();
    harness.tick(24);
    expect(harness.rendered.at(-1)).toEqual({
      elapsedSeconds: 0,
      animated: false,
      invalidated: true,
    });
    harness.tick(40);
    expect(harness.rendered.at(-1)?.animated).toBe(true);
  });

  it('暂停后停止空转，仍响应交互，恢复时不累计后台时间', () => {
    const harness = createHarness();
    harness.scheduler.setAnimating(true);
    harness.tick(40);
    harness.scheduler.setAnimating(false);
    harness.tick(50);
    expect(harness.pending.size).toBe(0);
    harness.scheduler.invalidate();
    harness.tick(10_000);
    expect(harness.rendered.at(-1)?.elapsedSeconds).toBe(0.04);
    harness.scheduler.setAnimating(true);
    harness.tick(10_040);
    expect(harness.rendered.at(-1)?.elapsedSeconds).toBe(0.08);
  });

  it('销毁会取消待绘制帧，后续失效与恢复均不能重新启动', () => {
    const harness = createHarness();
    harness.scheduler.setAnimating(true);
    harness.scheduler.destroy();
    harness.scheduler.invalidate();
    harness.scheduler.setAnimating(true);
    harness.tick(1000);
    expect(harness.pending.size).toBe(0);
    expect(harness.rendered).toEqual([]);
  });

  it('绘制回调中的新交互不会丢失，也不会排入重复帧', () => {
    const harness = createHarness(() => {
      if (harness.rendered.length === 1) {
        harness.scheduler.invalidate();
        harness.scheduler.invalidate();
      }
    });
    harness.scheduler.invalidate();
    harness.tick(0);
    expect(harness.pending.size).toBe(1);
    harness.tick(16);
    expect(harness.rendered).toHaveLength(2);
    expect(harness.pending.size).toBe(0);
  });
});
