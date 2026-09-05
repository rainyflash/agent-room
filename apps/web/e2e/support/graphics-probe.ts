import type { Page } from '@playwright/test';

type GraphicsSample = {
  readonly drawCalls: number;
  readonly frames: number;
  readonly triangles: number;
};

export async function installGraphicsProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const counter = { drawCalls: 0, triangles: 0 };
    Object.defineProperty(window, '__agentRoomGraphicsProbe', { value: counter });
    const record = (context: WebGLRenderingContext, mode: number, vertices: number): void => {
      counter.drawCalls += 1;
      if (mode === context.TRIANGLES) counter.triangles += vertices / 3;
      else if (mode === context.TRIANGLE_STRIP || mode === context.TRIANGLE_FAN)
        counter.triangles += Math.max(0, vertices - 2);
    };
    // 统计真实图形提交，避免只测 JavaScript 更新而漏掉软件渲染瓶颈。
    for (const context of [WebGLRenderingContext, WebGL2RenderingContext]) {
      const drawElements: unknown = Reflect.get(context.prototype, 'drawElements');
      const drawArrays: unknown = Reflect.get(context.prototype, 'drawArrays');
      if (typeof drawElements !== 'function' || typeof drawArrays !== 'function')
        throw new Error('图形上下文缺少绘制方法。');
      context.prototype.drawElements = function (
        this: WebGLRenderingContext | WebGL2RenderingContext,
        ...parameters: Parameters<WebGLRenderingContext['drawElements']>
      ): void {
        record(this, parameters[0], parameters[1]);
        Reflect.apply(drawElements, this, parameters);
      };
      context.prototype.drawArrays = function (
        this: WebGLRenderingContext | WebGL2RenderingContext,
        ...parameters: Parameters<WebGLRenderingContext['drawArrays']>
      ): void {
        record(this, parameters[0], parameters[2]);
        Reflect.apply(drawArrays, this, parameters);
      };
    }
  });
}

export async function graphicsSample(page: Page): Promise<GraphicsSample> {
  return await page.evaluate(() => {
    const state = window as Window & {
      readonly __agentRoomGraphicsProbe?: {
        readonly drawCalls: number;
        readonly triangles: number;
      };
    };
    const scene = document.querySelector<HTMLElement>('.lobby-scene__pixi');
    const drawCalls = state.__agentRoomGraphicsProbe?.drawCalls;
    const triangles = state.__agentRoomGraphicsProbe?.triangles;
    const frames = Number(scene?.dataset.agentRoomDrawnFrames);
    if (drawCalls === undefined || triangles === undefined || !Number.isFinite(frames))
      throw new Error('场景没有产生有效的图形提交遥测。');
    return { drawCalls, frames, triangles };
  });
}
