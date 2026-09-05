import type { Page } from '@playwright/test';

type GraphicsSample = { readonly drawCalls: number; readonly frames: number };

export async function installGraphicsProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const counter = { drawCalls: 0 };
    Object.defineProperty(window, '__agentRoomGraphicsProbe', { value: counter });
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
        counter.drawCalls += 1;
        Reflect.apply(drawElements, this, parameters);
      };
      context.prototype.drawArrays = function (
        this: WebGLRenderingContext | WebGL2RenderingContext,
        ...parameters: Parameters<WebGLRenderingContext['drawArrays']>
      ): void {
        counter.drawCalls += 1;
        Reflect.apply(drawArrays, this, parameters);
      };
    }
  });
}

export async function graphicsSample(page: Page): Promise<GraphicsSample> {
  return await page.evaluate(() => {
    const state = window as Window & {
      readonly __agentRoomGraphicsProbe?: { readonly drawCalls: number };
    };
    const scene = document.querySelector<HTMLElement>('.lobby-scene__pixi');
    const drawCalls = state.__agentRoomGraphicsProbe?.drawCalls;
    const frames =
      Number(scene?.dataset.agentRoomRenderSequence) +
      Number(scene?.dataset.agentRoomAnimationFrame ?? '0');
    if (drawCalls === undefined || !Number.isFinite(frames))
      throw new Error('场景没有产生有效的图形提交遥测。');
    return { drawCalls, frames };
  });
}
