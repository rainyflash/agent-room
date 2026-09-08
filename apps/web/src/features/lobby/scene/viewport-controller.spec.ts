import { describe, expect, it } from 'vitest';

import { ViewportController } from './viewport-controller';

describe('ViewportController', () => {
  it('从地图进入区域时恢复可辨认的人物大小，并保持真实目的地居中', () => {
    const controller = new ViewportController(
      { width: 10000, height: 7000 },
      { minimumScale: 0.3 },
    );
    controller.resize(1440, 800);
    const view = controller.focusArea(7500, 1750);
    expect(view.scale).toBe(1);
    expect(view.x + 7500 * view.scale).toBe(720);
    expect(view.y + 1750 * view.scale).toBe(400);
  });
  it('手机初始人物可辨认，适应房间按钮仍能恢复全景', () => {
    const controller = new ViewportController(
      { width: 1536, height: 1024 },
      { padding: 22, minimumScale: 0.22, compactInitialScale: 0.48 },
    );
    expect(controller.resize(390, 602).scale).toBeCloseTo(0.48, 8);
    const fitted = controller.reset();
    expect(fitted.scale).toBeLessThan(0.48);
    expect(1536 * fitted.scale).toBeLessThanOrEqual(390);
  });
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

  it.each([390, 1440])('选中人物时镜头放大，并让角色留在 %s 宽视口可操作区域', (width) => {
    const controller = new ViewportController({ height: 1500, width: 2600 });
    controller.resize(width, 900);
    const camera = controller.focusOn(1200, 450);
    const x = camera.x + 1200 * camera.scale;
    const y = camera.y + 450 * camera.scale;
    expect(camera.scale).toBeGreaterThanOrEqual(0.55);
    expect(x).toBeGreaterThan(20);
    expect(x).toBeLessThan(width - 20);
    if (width < 768) expect(y).toBe(36);
    else {
      expect(y).toBeGreaterThan(120);
      expect(y).toBeLessThan(600);
    }
  });

  it('靠近房间边缘的人物也保留详情展示空间，窗口或世界变化后仍保持定位', () => {
    const controller = new ViewportController(
      { height: 4608, width: 6144 },
      { minimumScale: 0.04 },
    );
    controller.resize(390, 602);
    controller.focusOn(5900, 4400);
    controller.updateWorld({ height: 4864, width: 6400 });
    const camera = controller.resize(360, 500);
    expect(camera.x + 5900 * camera.scale).toBe(180);
    expect(camera.y + 4400 * camera.scale).toBe(36);
    controller.panBy(0, 50);
    expect(controller.resize(390, 500).y + 4400 * camera.scale).not.toBe(36);
  });

  it('关闭人物详情后恢复正常浏览位置，地图跳转不被旧选中位置拉回', () => {
    const controller = new ViewportController({ width: 10000, height: 7000 });
    controller.resize(390, 600);
    controller.focusOn(5000, 3500);
    const released = controller.releaseFocus();
    expect(released.y + 3500 * released.scale).toBe(300);
    controller.focusOn(5000, 3500);
    const mapView = controller.focusArea(7000, 2000);
    expect(controller.releaseFocus()).toEqual(mapView);
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
