import type { Graphics, Renderer, Sprite, Texture } from 'pixi.js';
import { characterSeed } from '../../domain/room-floor';
import { characterBodyArt, characterStatusColor } from '../character-art';
import type { SceneCharacter } from '../scene-character';
import { shapeGraphics } from './shape-graphics';

type PixiModule = typeof import('pixi.js');
type CharacterTexture = {
  readonly texture: Texture;
  readonly x: number;
  readonly y: number;
};

export type CharacterParts = {
  readonly shadow: Sprite;
  readonly leftLeg: Sprite;
  readonly rightLeg: Sprite;
  readonly arms: Sprite;
  readonly marker: Sprite;
  readonly bubble: Sprite | null;
  readonly selectionRing: Sprite | null;
};

const partBuilders = {
  shadow: (pixi: PixiModule) =>
    new pixi.Graphics().ellipse(0, 1, 21, 8).fill({ color: '#696c4f', alpha: 0.24 }),
  leftLeg: (pixi: PixiModule) =>
    new pixi.Graphics()
      .roundRect(-11, -17, 9, 18, 3)
      .fill('#47564e')
      .roundRect(-13, -4, 12, 6, 2)
      .fill('#eee9d7'),
  rightLeg: (pixi: PixiModule) =>
    new pixi.Graphics()
      .roundRect(2, -17, 9, 18, 3)
      .fill('#47564e')
      .roundRect(2, -4, 12, 6, 2)
      .fill('#eee9d7'),
  arms: (pixi: PixiModule) =>
    new pixi.Graphics()
      .roundRect(-20, -33, 7, 22, 3)
      .fill('#dab493')
      .roundRect(13, -33, 7, 22, 3)
      .fill('#dab493'),
  bubble: (pixi: PixiModule) =>
    new pixi.Graphics()
      .roundRect(-10, -92, 23, 23, 8)
      .fill('#fff6d9')
      .stroke({ color: '#ccbb95', width: 1.5 }),
  selectionRing: (pixi: PixiModule) =>
    new pixi.Graphics()
      .ellipse(0, 1, 28, 12)
      .stroke({ color: '#fff8da', width: 4 })
      .ellipse(0, 1, 31, 14)
      .stroke({ color: '#769265', width: 2 }),
} satisfies Readonly<Record<string, (pixi: PixiModule) => Graphics>>;

/** 纹理由场景统一释放；每个角色仅拥有自己的 Sprite。 */
export class CharacterTextureCache {
  readonly #pixi: PixiModule;
  readonly #renderer: Pick<Renderer, 'generateTexture'>;
  readonly #textures = new Map<string, CharacterTexture>();
  #destroyed = false;

  constructor(pixi: PixiModule, renderer: Pick<Renderer, 'generateTexture'>) {
    this.#pixi = pixi;
    this.#renderer = renderer;
  }

  createBody(node: SceneCharacter): Sprite {
    // 外观只使用种子对 6、4、3 的余数，同类角色最多需要 12 张纹理。
    const key = `${node.kind}:${String(characterSeed(node.characterId) % 12)}`;
    const cached = this.#texture(key, () =>
      shapeGraphics(this.#pixi, characterBodyArt(node.characterId, node.kind)),
    );
    const sprite = new this.#pixi.Sprite(cached.texture);
    sprite.position.set(cached.x, cached.y);
    sprite.eventMode = 'none';
    return sprite;
  }

  createParts(node: SceneCharacter, selected: boolean): CharacterParts {
    // 先生成完整纹理组，失败时不会留下尚未交给视图管理的 Sprite。
    const shadow = this.#partTexture('shadow');
    const leftLeg = this.#partTexture('leftLeg');
    const rightLeg = this.#partTexture('rightLeg');
    const arms = this.#partTexture('arms');
    const marker = this.#texture(`marker:${node.status}`, () =>
      new this.#pixi.Graphics()
        .circle(19, -61, 6)
        .fill(characterStatusColor[node.status])
        .stroke({ color: '#fff7e2', width: 2 }),
    );
    const bubble =
      node.status === 'waiting_input' || node.status === 'blocked'
        ? this.#partTexture('bubble')
        : null;
    const selectionRing = selected ? this.#partTexture('selectionRing') : null;
    return {
      shadow: this.#partSprite(shadow),
      leftLeg: this.#partSprite(leftLeg),
      rightLeg: this.#partSprite(rightLeg),
      arms: this.#partSprite(arms),
      marker: this.#partSprite(marker),
      bubble: bubble === null ? null : this.#partSprite(bubble),
      selectionRing: selectionRing === null ? null : this.#partSprite(selectionRing),
    };
  }

  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    for (const { texture } of this.#textures.values()) texture.destroy(true);
    this.#textures.clear();
  }

  #partTexture(part: keyof typeof partBuilders): CharacterTexture {
    return this.#texture(`part:${part}`, () => partBuilders[part](this.#pixi));
  }

  #partSprite(cached: CharacterTexture): Sprite {
    const sprite = new this.#pixi.Sprite(cached.texture);
    // 保持旧几何的局部原点，腿部位移和手臂旋转不受纹理裁剪区域影响。
    sprite.pivot.set(-cached.x, -cached.y);
    sprite.eventMode = 'none';
    return sprite;
  }

  #texture(key: string, build: () => Graphics): CharacterTexture {
    if (this.#destroyed) throw new Error('角色纹理缓存已销毁。');
    let cached = this.#textures.get(key);
    if (cached === undefined) {
      cached = this.#createTexture(build());
      this.#textures.set(key, cached);
    }
    return cached;
  }

  #createTexture(graphics: Graphics): CharacterTexture {
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
