import type { Container, Graphics, Renderer, Sprite, Texture } from 'pixi.js';
import {
  characterFill,
  characterShadow,
  humanMarker,
  sceneInk,
  selectionRing,
} from '../scene-style';
import { characterFoot, characterSize, characterVariant, spriteCell } from '../studio-assets';
import type { SceneCharacter } from '../scene-character';

type PixiModule = typeof import('pixi.js');
type CharacterTexture = {
  readonly texture: Texture;
  readonly x: number;
  readonly y: number;
};

export type CharacterParts = {
  readonly shadow: Sprite;
  readonly marker: Sprite;
  readonly selectionRing: Sprite | null;
};

const partBuilders = {
  shadow: (pixi: PixiModule) =>
    new pixi.Graphics()
      .ellipse(0, 1, 21, 8)
      .fill({ color: characterShadow.color, alpha: characterShadow.alpha }),
  // 墨色外圈垫底、晴黄内圈压上：在浅色地砖上也一眼能看出选中了谁。
  selectionRing: (pixi: PixiModule) =>
    new pixi.Graphics()
      .ellipse(0, 1, 30, 13)
      .stroke({ color: selectionRing.outer, width: selectionRing.outerWidth })
      .ellipse(0, 1, 30, 13)
      .stroke({ color: selectionRing.inner, width: selectionRing.innerWidth }),
} satisfies Readonly<Record<string, (pixi: PixiModule) => Graphics>>;

/** 纹理由场景统一释放；每个角色仅拥有自己的 Sprite。 */
export class CharacterTextureCache {
  readonly #pixi: PixiModule;
  readonly #renderer: Pick<Renderer, 'generateTexture'>;
  readonly #atlas: Texture;
  readonly #frames: Texture[] = [];
  readonly #textures = new Map<string, CharacterTexture>();
  #destroyed = false;

  constructor(pixi: PixiModule, renderer: Pick<Renderer, 'generateTexture'>, atlas: Texture) {
    this.#pixi = pixi;
    this.#renderer = renderer;
    this.#atlas = atlas;
  }

  createBody(node: SceneCharacter, walking = false): Sprite {
    const variant = characterVariant(node.characterId);
    const cached = this.#texture(
      node.kind === 'human'
        ? 'body:human'
        : `body:${String(variant)}:${walking ? 'walking' : 'standing'}`,
      () => {
        if (node.kind === 'human')
          return new this.#pixi.Graphics()
            .poly([-28, -32, -14, -57, 14, -57, 28, -32, 14, -7, -14, -7])
            .fill(humanMarker.fill)
            .circle(0, -32, 10)
            .fill(humanMarker.mark);
        const frame = new this.#pixi.Texture({
          source: this.#atlas.source,
          frame: new this.#pixi.Rectangle(
            variant * spriteCell.width,
            walking ? spriteCell.height : 0,
            spriteCell.width,
            spriteCell.height,
          ),
        });
        this.#frames.push(frame);
        const sprite = new this.#pixi.Sprite(frame);
        const foot = characterFoot(node.characterId, walking);
        sprite.position.set(
          (-foot.x * characterSize.width) / spriteCell.width,
          (-foot.y * characterSize.height) / spriteCell.height,
        );
        sprite.width = characterSize.width;
        sprite.height = characterSize.height;
        const body = new this.#pixi.Container();
        body.addChild(sprite);
        return body;
      },
    );
    const sprite = new this.#pixi.Sprite(cached.texture);
    sprite.position.set(cached.x, cached.y);
    sprite.eventMode = 'none';
    return sprite;
  }

  createParts(node: SceneCharacter, selected: boolean): CharacterParts {
    // 先生成完整纹理组，失败时不会留下尚未交给视图管理的 Sprite。
    const shadow = this.#partTexture('shadow');
    const fill = characterFill(node);
    const marker = this.#texture(`marker:${fill}`, () =>
      new this.#pixi.Graphics()
        .circle(30, -70, 4.5)
        .fill(fill)
        .stroke({ color: sceneInk, width: 2 }),
    );
    const selectionRing = selected ? this.#partTexture('selectionRing') : null;
    return {
      shadow: this.#partSprite(shadow),
      marker: this.#partSprite(marker),
      selectionRing: selectionRing === null ? null : this.#partSprite(selectionRing),
    };
  }

  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    for (const { texture } of this.#textures.values()) texture.destroy(true);
    this.#textures.clear();
    for (const frame of this.#frames) frame.destroy(false);
    this.#frames.length = 0;
    this.#atlas.destroy(true);
  }

  #partTexture(part: keyof typeof partBuilders): CharacterTexture {
    return this.#texture(`part:${part}`, () => partBuilders[part](this.#pixi));
  }

  #partSprite(cached: CharacterTexture): Sprite {
    const sprite = new this.#pixi.Sprite(cached.texture);
    // Indicators keep the character's foot as their local origin.
    sprite.pivot.set(-cached.x, -cached.y);
    sprite.eventMode = 'none';
    return sprite;
  }

  #texture(key: string, build: () => Container): CharacterTexture {
    if (this.#destroyed) throw new Error('角色纹理缓存已销毁。');
    let cached = this.#textures.get(key);
    if (cached === undefined) {
      cached = this.#createTexture(build());
      this.#textures.set(key, cached);
    }
    return cached;
  }

  #createTexture(graphics: Container): CharacterTexture {
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
      graphics.destroy({ children: true, context: true });
    }
  }
}
