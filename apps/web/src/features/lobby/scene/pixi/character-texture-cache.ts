import type { Container, Graphics, Renderer, Sprite, Texture } from 'pixi.js';
import { characterStatusColor } from '../character-art';
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
  readonly bubble: Sprite | null;
  readonly selectionRing: Sprite | null;
};

const partBuilders = {
  shadow: (pixi: PixiModule) =>
    new pixi.Graphics().ellipse(0, 1, 21, 8).fill({ color: '#696c4f', alpha: 0.24 }),
  waitingBubble: (pixi: PixiModule) => statusBubble(pixi, 'waiting_input'),
  blockedBubble: (pixi: PixiModule) => statusBubble(pixi, 'blocked'),
  selectionRing: (pixi: PixiModule) =>
    new pixi.Graphics()
      .ellipse(0, 1, 28, 12)
      .stroke({ color: '#fff8da', width: 4 })
      .ellipse(0, 1, 31, 14)
      .stroke({ color: '#769265', width: 2 }),
} satisfies Readonly<Record<string, (pixi: PixiModule) => Graphics>>;

function statusBubble(pixi: PixiModule, status: 'waiting_input' | 'blocked'): Graphics {
  const graphic = new pixi.Graphics()
    .roundRect(25, -106, 23, 23, 8)
    .fill('#fff6d9')
    .stroke({ color: '#ccbb95', width: 1.5 });
  if (status === 'blocked') {
    graphic.moveTo(37, -101).lineTo(37, -94);
  } else {
    graphic
      .moveTo(33, -98)
      .bezierCurveTo(33, -103, 42, -103, 42, -98)
      .bezierCurveTo(42, -96, 37, -96, 37, -93);
  }
  return graphic.stroke({ color: '#74502e', width: 2 }).circle(37, -89, 1.2).fill('#74502e');
}

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
            .fill('#173544')
            .circle(0, -32, 10)
            .fill('#ffffff');
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
    const marker = this.#texture(`marker:${node.status}`, () =>
      new this.#pixi.Graphics()
        .circle(30, -70, 4)
        .fill(characterStatusColor[node.status])
        .stroke({ color: '#fff7e2', width: 2 }),
    );
    const bubble =
      node.status === 'waiting_input' || node.status === 'blocked'
        ? this.#partTexture(node.status === 'blocked' ? 'blockedBubble' : 'waitingBubble')
        : null;
    const selectionRing = selected ? this.#partTexture('selectionRing') : null;
    return {
      shadow: this.#partSprite(shadow),
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
