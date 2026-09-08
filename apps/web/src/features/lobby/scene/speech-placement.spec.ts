import { describe, expect, it } from 'vitest';
import { placeSpeech, type SpeechBounds } from './speech-placement';

describe('拥挤房间的气泡排布', () => {
  it('手机顶部同时发言不会覆盖已有气泡或挤出屏幕', () => {
    const occupied: SpeechBounds[] = [];
    for (let index = 0; index < 3; index += 1) {
      const placed = placeSpeech(
        { x: 190, y: 140 },
        { width: 175, height: 70 },
        { width: 390, height: 844 },
        occupied,
      );
      if (placed === null) continue;
      expect(placed.y).toBeGreaterThanOrEqual(40);
      expect(placed.x + placed.width).toBeLessThanOrEqual(378);
      for (const prior of occupied)
        expect(
          placed.y >= prior.y + prior.height ||
            placed.y + placed.height <= prior.y ||
            placed.x >= prior.x + prior.width ||
            placed.x + placed.width <= prior.x,
        ).toBe(true);
      occupied.push(placed);
    }
    expect(occupied.length).toBeGreaterThan(1);
  });

  it('场景顶部仍可见的人物可以发言，不会重复扣除页面顶栏而隐藏气泡', () => {
    const placed = placeSpeech(
      { x: 190, y: 105 },
      { width: 175, height: 58 },
      { width: 390, height: 602 },
      [],
    );
    expect(placed).not.toBeNull();
    expect(placed?.y).toBeGreaterThanOrEqual(40);
    expect((placed?.y ?? 0) + (placed?.height ?? 0)).toBeLessThanOrEqual(105);
  });

  it('画面外人物和放不下的长气泡不抢占顶部或底部操作区', () => {
    expect(
      placeSpeech({ x: -10, y: 300 }, { width: 175, height: 70 }, { width: 390, height: 844 }, []),
    ).toBeNull();
    expect(
      placeSpeech({ x: 190, y: 140 }, { width: 175, height: 300 }, { width: 390, height: 320 }, []),
    ).toBeNull();
  });
});
