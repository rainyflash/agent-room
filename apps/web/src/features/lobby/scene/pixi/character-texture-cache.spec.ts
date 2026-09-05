import * as pixi from 'pixi.js';
import type { GenerateTextureOptions, Renderer } from 'pixi.js';
import { describe, expect, it, vi } from 'vitest';
import { characterSeed } from '../../domain/room-floor';
import type { SceneCharacter } from '../scene-character';
import { CharacterTextureCache } from './character-texture-cache';

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
    const cache = new CharacterTextureCache(pixi, generation);
    const node = character(0);
    const matching = Array.from({ length: 200 }, (_, index) => character(index + 1)).find(
      (candidate) =>
        characterSeed(candidate.characterId) % 12 === characterSeed(node.characterId) % 12,
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

  it('每类 200 个角色只生成 12 种外观，区分人类与 Agent', () => {
    const generation = renderer();
    const cache = new CharacterTextureCache(pixi, generation);
    const agents = Array.from({ length: 200 }, (_, index) => cache.createBody(character(index)));
    const humans = Array.from({ length: 200 }, (_, index) =>
      cache.createBody(character(index, 'human')),
    );

    expect(new Set(agents.map((sprite) => sprite.texture)).size).toBe(12);
    expect(new Set(humans.map((sprite) => sprite.texture)).size).toBe(12);
    expect(generation.generateTexture).toHaveBeenCalledTimes(24);
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
    const cache = new CharacterTextureCache(pixi, generation);
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
    const cache = new CharacterTextureCache(pixi, generation);
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
});
