import * as pixi from 'pixi.js';
import type { GenerateTextureOptions, Renderer } from 'pixi.js';
import { describe, expect, it, vi } from 'vitest';
import { characterSeed } from '../../domain/room-floor';
import type { SceneCharacter } from '../scene-character';
import { CharacterTextureCache } from './character-texture-cache';
import { createAgentNodeView } from './agent-node-view';

function character(index: number, kind: SceneCharacter['kind'] = 'agent'): SceneCharacter {
  return {
    characterId: `character-${String(index)}`,
    matrixUserId: `@character-${String(index)}:test`,
    displayName: `角色 ${String(index)}`,
    kind,
    isSelf: false,
    status: kind === 'human' ? 'present' : 'idle',
    radius: 26,
    x: 0,
    y: 0,
  };
}

function textureRequest(
  input: Parameters<Renderer['generateTexture']>[0] | undefined,
): GenerateTextureOptions {
  if (input === undefined || input instanceof pixi.Container)
    throw new Error('纹理生成必须指定区域和分辨率。');
  return input;
}

function renderer() {
  return {
    generateTexture: vi.fn<Renderer['generateTexture']>((input) => {
      const request = textureRequest(input);
      return pixi.RenderTexture.create({
        width: request.frame?.width ?? 1,
        height: request.frame?.height ?? 1,
        resolution: request.resolution ?? 1,
      });
    }),
  };
}

describe('场景共享角色纹理', () => {
  it('相同外观复用纹理，角色位置和销毁保持独立', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const node = character(0);
    const matching = Array.from({ length: 200 }, (_, index) => character(index + 1)).find(
      (candidate) =>
        characterSeed(candidate.characterId) % 4 === characterSeed(node.characterId) % 4,
    );
    if (matching === undefined) throw new Error('缺少相同外观的测试角色。');
    const first = cache.createBody(node);
    const second = cache.createBody(matching);
    const texture = first.texture;
    const release = vi.spyOn(texture, 'destroy');
    const secondPosition = { x: second.x, y: second.y };

    expect(generation.generateTexture).toHaveBeenCalledTimes(1);
    expect(first).not.toBe(second);
    expect(second.texture).toBe(texture);
    first.position.set(300, 400);
    expect({ x: second.x, y: second.y }).toEqual(secondPosition);
    first.destroy();
    expect(release).not.toHaveBeenCalled();
    expect(second.texture.destroyed).toBe(false);
    second.destroy();
    cache.destroy();
    cache.destroy();
    expect(release).toHaveBeenCalledExactlyOnceWith(true);
    expect(() => cache.createBody(node)).toThrow('角色纹理缓存已销毁。');
  });

  it('400 个角色复用四种人物素材，身份由名字和状态区分', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const agents = Array.from({ length: 200 }, (_, index) => cache.createBody(character(index)));
    const humans = Array.from({ length: 200 }, (_, index) =>
      cache.createBody(character(index, 'human')),
    );

    expect(new Set(agents.map((sprite) => sprite.texture)).size).toBe(4);
    expect(new Set(humans.map((sprite) => sprite.texture)).size).toBe(1);
    expect(generation.generateTexture).toHaveBeenCalledTimes(5);
    expect(agents[0]?.texture).not.toBe(humans[0]?.texture);
    for (const sprite of [...agents, ...humans]) sprite.destroy();
    cache.destroy();
  });

  it('纹理区域覆盖真实图形并保留安全边距和原始坐标', () => {
    const generation = renderer();
    generation.generateTexture.mockImplementation((input) => {
      const request = textureRequest(input);
      const bounds = request.target.getLocalBounds();
      const frame = request.frame;
      if (frame === undefined) throw new Error('缺少角色纹理区域。');
      expect(frame.x).toBeLessThanOrEqual(bounds.minX - 2);
      expect(frame.y).toBeLessThanOrEqual(bounds.minY - 2);
      expect(frame.right).toBeGreaterThanOrEqual(bounds.maxX + 2);
      expect(frame.bottom).toBeGreaterThanOrEqual(bounds.maxY + 2);
      expect(request.resolution).toBe(2);
      expect(request.antialias).toBe(true);
      return pixi.RenderTexture.create({ width: frame.width, height: frame.height, resolution: 2 });
    });
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const sprite = cache.createBody(character(0));
    const request = textureRequest(generation.generateTexture.mock.calls[0]?.[0]);

    expect({ x: sprite.x, y: sprite.y }).toEqual({ x: request.frame?.x, y: request.frame?.y });
    expect(request.target.destroyed).toBe(true);
    expect(sprite.eventMode).toBe('none');
    sprite.destroy();
    cache.destroy();
  });

  it('纹理生成失败会释放临时图形，重试仍能生成同一角色', () => {
    const generation = renderer();
    generation.generateTexture.mockImplementationOnce(() => {
      throw new Error('纹理生成失败');
    });
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const node = character(0);

    expect(() => cache.createBody(node)).toThrow('纹理生成失败');
    const failed = textureRequest(generation.generateTexture.mock.calls[0]?.[0]);
    expect(failed.target.destroyed).toBe(true);
    const sprite = cache.createBody(node);
    expect(generation.generateTexture).toHaveBeenCalledTimes(2);
    expect(sprite.texture.destroyed).toBe(false);
    sprite.destroy();
    cache.destroy();
  });

  it('各类角色与状态的全部部件保持有界共享', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const statuses = [
      'idle',
      'working',
      'completed',
      'waiting_input',
      'blocked',
      'offline',
    ] as const;
    const sprites: pixi.Sprite[] = [];
    for (const kind of ['agent', 'human'] as const) {
      for (let index = 0; index < 200; index += 1) {
        const node: SceneCharacter = {
          ...character(index, kind),
          status: kind === 'human' ? 'present' : (statuses[index % statuses.length] ?? 'idle'),
        };
        sprites.push(cache.createBody(node));
        const parts = cache.createParts(node, index % 2 === 0);
        for (const sprite of Object.values(parts)) {
          if (sprite !== null) sprites.push(sprite);
        }
      }
    }

    // 四种机器人、一种人类标记、四种指示部件和七种状态点，不随人数增长。
    expect(generation.generateTexture).toHaveBeenCalledTimes(16);
    expect(new Set(sprites.map((sprite) => sprite.texture)).size).toBe(16);
    const textures = [...new Set(sprites.map((sprite) => sprite.texture))];
    const releases = textures.map((texture) => vi.spyOn(texture, 'destroy'));
    const sources = textures.map((texture) => texture.source);
    for (const sprite of sprites) sprite.destroy();
    expect(releases.every((release) => release.mock.calls.length === 0)).toBe(true);
    cache.destroy();
    cache.destroy();
    expect(sources.every((source) => source.destroyed)).toBe(true);
    for (const release of releases) expect(release).toHaveBeenCalledExactlyOnceWith(true);
    expect(() => cache.createParts(character(0), false)).toThrow('角色纹理缓存已销毁。');
  });

  it('相同状态共享完整气泡纹理，不同符号区分且每个 Sprite 独立', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const waiting = cache.createParts({ ...character(0), status: 'waiting_input' }, true);
    const alsoWaiting = cache.createParts({ ...character(3), status: 'waiting_input' }, true);
    const blocked = cache.createParts({ ...character(1), status: 'blocked' }, true);
    const idle = cache.createParts(character(2), false);

    expect(waiting.shadow).not.toBe(blocked.shadow);
    expect(waiting.shadow.texture).toBe(blocked.shadow.texture);
    expect(waiting.bubble?.texture).toBe(alsoWaiting.bubble?.texture);
    expect(waiting.bubble?.texture).not.toBe(blocked.bubble?.texture);
    expect(waiting.selectionRing?.texture).toBe(blocked.selectionRing?.texture);
    expect(waiting.marker.texture).not.toBe(blocked.marker.texture);
    expect(idle.bubble).toBeNull();
    expect(idle.selectionRing).toBeNull();
    for (const parts of [waiting, alsoWaiting, blocked, idle]) {
      for (const sprite of Object.values(parts)) sprite?.destroy();
    }
    cache.destroy();
  });

  it('人物变换不影响共享纹理的其他实例', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const first = cache.createBody(character(0));
    const second = cache.createBody(character(0));
    const original = { x: second.x, y: second.y, scale: second.scale.x };
    first.position.set(40, 60);
    first.scale.set(1.5);
    expect({ x: second.x, y: second.y, scale: second.scale.x }).toEqual(original);
    expect(first.texture).toBe(second.texture);
    first.destroy();
    second.destroy();
    cache.destroy();
  });

  it('人物视图保持位置与悬停深度，不修改场景分配的排序值', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    const node = character(0);
    const parts = cache.createParts(node, false);
    const body = cache.createBody(node);
    const invalidate = vi.fn();
    const view = createAgentNodeView(pixi, {
      node,
      body,
      parts,
      selected: false,
      detail: 'distant',
      onInvalidate: invalidate,
      onSelect: () => undefined,
    });
    const pose = { x: 12, y: 34, stride: 3.5, facing: -1, moving: true };
    view.container.zIndex = 7;
    view.animate(pose);
    expect(view.depth).toBe(34);
    expect(view.container.zIndex).toBe(7);
    const pointer = new pixi.FederatedPointerEvent(new pixi.EventBoundary());
    view.container.emit('pointerover', pointer);
    expect(invalidate).toHaveBeenCalledTimes(1);
    view.animate(pose);
    expect(view.depth).toBe(10000);
    expect(view.container.zIndex).toBe(7);
    view.container.emit('pointerout', pointer);
    expect(invalidate).toHaveBeenCalledTimes(2);
    view.animate(pose);
    expect(view.depth).toBe(34);
    const texture = parts.shadow.texture;
    view.destroy();
    expect(texture.destroyed).toBe(false);
    cache.destroy();
    expect(texture.destroyed).toBe(true);
  });

  it('部件生成中断时，临时图形和已缓存的纹理仍能完整释放', () => {
    const generation = renderer();
    generation.generateTexture
      .mockImplementationOnce(() => pixi.RenderTexture.create({ width: 48, height: 20 }))
      .mockImplementationOnce(() => {
        throw new Error('部件纹理生成失败');
      });
    const cache = new CharacterTextureCache(
      pixi,
      generation,
      pixi.RenderTexture.create({ width: 1536, height: 1024 }),
    );
    expect(() => cache.createParts(character(0), true)).toThrow('部件纹理生成失败');
    const created: unknown = generation.generateTexture.mock.results[0]?.value;
    if (!(created instanceof pixi.Texture)) throw new Error('缺少已生成的部件纹理。');
    for (const [input] of generation.generateTexture.mock.calls)
      expect(textureRequest(input).target.destroyed).toBe(true);
    expect(created.destroyed).toBe(false);
    cache.destroy();
    expect(created.destroyed).toBe(true);
  });
});
