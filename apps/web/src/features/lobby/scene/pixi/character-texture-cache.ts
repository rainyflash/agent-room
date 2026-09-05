import type { Renderer, Sprite, Texture } from 'pixi.js';
import { characterSeed } from '../../domain/room-floor';
import { characterBodyArt } from '../character-art';
import type { SceneCharacter } from '../scene-character';
import { shapeGraphics } from './shape-graphics';

type PixiModule = typeof import('pixi.js');
type BodyTexture = {
  readonly texture: Texture;
  readonly x: number;
  readonly y: number;
};

/** 纹理由场景统一释放；每个角色仅拥有自己的 Sprite。 */
export class CharacterTextureCache {
  readonly #pixi: PixiModule;
  readonly #renderer: Pick<Renderer, 'generateTexture'>;
  readonly #textures = new Map<string, BodyTexture>();
  #destroyed = false;

  constructor(pixi: PixiModule, renderer: Pick<Renderer, 'generateTexture'>) {
    this.#pixi = pixi;
    this.#renderer = renderer;
  }

  createBody(node: SceneCharacter): Sprite {
    if (this.#destroyed) throw new Error('角色纹理缓存已销毁。');
    // 外观只使用种子对 6、4、3 的余数，同类角色最多需要 12 张纹理。
    const key = `${node.kind}:${String(characterSeed(node.characterId) % 12)}`;
    let cached = this.#textures.get(key);
    if (cached === undefined) {
      cached = this.#createTexture(node);
      this.#textures.set(key, cached);
    }
    const sprite = new this.#pixi.Sprite(cached.texture);
    sprite.position.set(cached.x, cached.y);
    sprite.eventMode = 'none';
    return sprite;
  }

  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    for (const { texture } of this.#textures.values()) texture.destroy(true);
    this.#textures.clear();
  }

  #createTexture(node: SceneCharacter): BodyTexture {
    const graphics = shapeGraphics(this.#pixi, characterBodyArt(node.characterId, node.kind));
    try {
      const bounds = graphics.getLocalBounds();
      const x = Math.floor(bounds.minX) - 2;
      const y = Math.floor(bounds.minY) - 2;
      const frame = new this.#pixi.Rectangle(
        x,
        y,
        Math.ceil(bounds.maxX) + 2 - x,
        Math.ceil(bounds.maxY) + 2 - y,
      );
      const texture = this.#renderer.generateTexture({
        target: graphics,
        frame,
        resolution: 2,
        antialias: true,
      });
      return { texture, x, y };
    } finally {
      graphics.destroy({ context: true });
    }
  }
}
