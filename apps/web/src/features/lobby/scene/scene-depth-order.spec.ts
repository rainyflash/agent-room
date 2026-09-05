import { describe, expect, it } from 'vitest';
import { SceneDepthOrder } from './scene-depth-order';

function depthTarget() {
  let rank = -1;
  let writes = 0;
  return {
    get zIndex() {
      return rank;
    },
    set zIndex(next: number) {
      rank = next;
      writes += 1;
    },
    get writes() {
      return writes;
    },
  };
}

describe('SceneDepthOrder', () => {
  it('人物在家具前后移动时保留精确遮挡，相对顺序未变时不改渲染层级', () => {
    const order = new SceneDepthOrder();
    const character = depthTarget();
    const furniture = depthTarget();
    order.set(character, 90);
    order.set(furniture, 100);
    order.apply();
    expect(character.zIndex).toBeLessThan(furniture.zIndex);
    order.set(character, 99.999);
    order.apply();
    expect(character.writes).toBe(1);
    expect(furniture.writes).toBe(1);
    order.set(character, 100.001);
    order.apply();
    expect(character.zIndex).toBeGreaterThan(furniture.zIndex);
    expect(character.writes).toBe(2);
  });

  it('删除、重新加入和清空不会保留已销毁人物', () => {
    const order = new SceneDepthOrder();
    const target = depthTarget();
    order.set(target, 10);
    order.delete(target);
    order.apply();
    expect(target.writes).toBe(0);
    order.set(target, 10);
    order.apply();
    expect(target.writes).toBe(1);
    order.clear();
    order.apply();
    expect(target.writes).toBe(1);
  });
});
