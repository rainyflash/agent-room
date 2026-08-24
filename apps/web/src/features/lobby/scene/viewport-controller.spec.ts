import { describe, expect, it } from 'vitest';

import { ViewportController } from './viewport-controller';

describe('ViewportController', () => {
  it('首次尺寸确定时完整容纳世界并保持居中', () => {
    const controller = new ViewportController({ height: 1_000, width: 2_000 }, { padding: 50 });

    expect(controller.resize(1_100, 600)).toEqual({ scale: 0.5, x: 50, y: 50 });
    expect(controller.viewport()).toEqual({
      height: 1_200,
      width: 2_200,
      x: -100,
      y: -100,
      zoom: 0.5,
    });
  });

  it('缩放围绕光标锚点且平移被世界边界约束', () => {
    const controller = new ViewportController({ height: 1_000, width: 2_000 }, { padding: 0 });
    controller.resize(1_000, 500);

    expect(controller.zoomBy(2, 250, 125)).toEqual({ scale: 1, x: -250, y: -125 });
    expect(controller.panBy(10_000, 10_000)).toEqual({ scale: 1, x: 0, y: 0 });
    expect(controller.panBy(-10_000, -10_000)).toEqual({
      scale: 1,
      x: -1_000,
      y: -500,
    });
  });

  it('窗口变化保持原世界中心并拒绝非有限输入', () => {
    const controller = new ViewportController({ height: 1_000, width: 2_000 }, { padding: 0 });
    controller.resize(1_000, 500);
    controller.zoomBy(2);

    expect(controller.resize(800, 400)).toEqual({ scale: 1, x: -600, y: -300 });
    expect(controller.zoomBy(Number.NaN)).toEqual({ scale: 1, x: -600, y: -300 });
    expect(controller.panBy(Number.POSITIVE_INFINITY, Number.NaN)).toEqual({
      scale: 1,
      x: -600,
      y: -300,
    });
  });
});
