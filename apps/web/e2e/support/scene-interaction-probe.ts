import type { JSHandle, Page } from '@playwright/test';

export type SceneInteractionSample = {
  readonly renderMilliseconds: number;
  readonly scheduleMilliseconds: number;
  readonly updateMilliseconds: number;
  readonly wheelIndex: number;
};

type SceneInteractionProbe = {
  dispose(): void;
  waitForSample(index: number): Promise<SceneInteractionSample>;
};

export async function createSceneInteractionProbe(
  page: Page,
): Promise<JSHandle<SceneInteractionProbe>> {
  return await page.evaluateHandle(() => {
    const responseTimeoutMilliseconds = 1_000;
    const canvas = document.querySelector<HTMLCanvasElement>('.lobby-scene__canvas');
    const host = canvas?.closest<HTMLElement>('.lobby-scene__pixi');
    if (canvas === null || host === null || host === undefined)
      throw new Error('大厅场景缺少性能遥测宿主。');

    const samples: SceneInteractionSample[] = [];
    let failure: Error | null = null;
    let pending: { readonly sequence: number; readonly startedAt: number } | null = null;
    let timer: number | undefined;
    let waiting: {
      readonly index: number;
      readonly reject: (error: Error) => void;
      readonly resolve: (sample: SceneInteractionSample) => void;
    } | null = null;

    const fail = (error: Error): void => {
      failure = error;
      window.clearTimeout(timer);
      timer = undefined;
      pending = null;
      waiting?.reject(error);
      waiting = null;
    };
    const onWheel = (event: WheelEvent): void => {
      if (failure !== null) return;
      if (!event.isTrusted || pending !== null) {
        fail(new Error('场景采样要求每次真实滚轮输入独占一个完成的渲染。'));
        return;
      }
      const sequence = Number(host.dataset.agentRoomRenderSequence ?? Number.NaN);
      if (!Number.isFinite(sequence)) {
        fail(new Error('大厅渲染序列无效。'));
        return;
      }
      pending = { sequence, startedAt: performance.now() };
      // 应用响应预算从实际输入开始，避免把协议等待误算成绘制耗时。
      timer = window.setTimeout(() => {
        fail(new Error('滚轮输入在 1 秒内没有完成渲染。'));
      }, responseTimeoutMilliseconds);
    };
    const observer = new MutationObserver(() => {
      if (pending === null || failure !== null) return;
      const sequence = Number(host.dataset.agentRoomRenderSequence ?? Number.NaN);
      if (!Number.isFinite(sequence) || sequence <= pending.sequence) return;
      const scheduleMilliseconds = performance.now() - pending.startedAt;
      // 微任务可能抢先于超时回调执行，仍须按真实时间拒绝超预算结果。
      if (scheduleMilliseconds > responseTimeoutMilliseconds) {
        fail(new Error('滚轮输入在 1 秒内没有完成渲染。'));
        return;
      }
      const renderMilliseconds = Number(host.dataset.agentRoomRenderMilliseconds ?? Number.NaN);
      const updateMilliseconds = Number(host.dataset.agentRoomUpdateMilliseconds ?? Number.NaN);
      if (![renderMilliseconds, updateMilliseconds].every(Number.isFinite)) {
        fail(new Error('大厅交互没有产生完整的性能遥测。'));
        return;
      }
      // 在对应帧完成时冻结数据，后续动画不得覆盖这次输入的结果。
      const sample: SceneInteractionSample = {
        renderMilliseconds,
        scheduleMilliseconds,
        updateMilliseconds,
        wheelIndex: samples.length,
      };
      window.clearTimeout(timer);
      timer = undefined;
      pending = null;
      samples.push(sample);
      if (waiting !== null) {
        if (waiting.index !== sample.wheelIndex) {
          fail(new Error('滚轮输入与采样序号不一致。'));
          return;
        }
        waiting.resolve(sample);
        waiting = null;
      }
    });
    canvas.addEventListener('wheel', onWheel, { capture: true, passive: true });
    observer.observe(host, {
      attributeFilter: ['data-agent-room-render-sequence'],
      attributes: true,
    });

    return {
      dispose(): void {
        observer.disconnect();
        canvas.removeEventListener('wheel', onWheel, true);
        fail(new Error('大厅交互采样已结束。'));
      },
      async waitForSample(index: number): Promise<SceneInteractionSample> {
        if (failure !== null) throw failure;
        const sample = samples[index];
        if (sample !== undefined) return sample;
        if (index !== samples.length || waiting !== null)
          throw new Error('大厅交互采样必须逐次等待，不能跳过或合并输入。');
        return await new Promise<SceneInteractionSample>((resolve, reject) => {
          waiting = { index, reject, resolve };
        });
      },
    };
  });
}
