import type { SceneCharacter } from '../scene-character';
import { characterLabel } from '../character-labels';
import type { Container, FederatedPointerEvent, Sprite } from 'pixi.js';
import type { LobbySceneDetail } from '@/features/lobby/domain/scene-projection';
import type { CharacterPose } from '../character-motion';
import type { CharacterParts } from './character-texture-cache';
import {
  estimatedTextWidth,
  nameplate,
  sceneFont,
  sceneInk,
  statusSticker,
  statusStickerFill,
} from '../scene-style';

type PixiModule = typeof import('pixi.js');
const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
export type AgentNodeViewOptions = {
  readonly body: Sprite;
  readonly walkingBody?: Sprite;
  readonly parts: CharacterParts;
  readonly detail: LobbySceneDetail;
  readonly node: SceneCharacter;
  readonly onInvalidate: () => void;
  readonly onSelect: (agentId: string) => void;
  readonly selected: boolean;
  readonly showName?: boolean;
  readonly statusLabel?: string | undefined;
};
export type AgentCharacterView = {
  readonly container: Container;
  readonly depth: number;
  animate(pose: CharacterPose): void;
  destroy(): void;
};

export function createAgentNodeView(
  pixi: PixiModule,
  options: AgentNodeViewOptions,
): AgentCharacterView {
  const { node, selected, parts } = options;
  const container = new pixi.Container();
  const size = Math.max(0.83, node.radius / 27);
  const character = new pixi.Container();
  character.scale.set(size);
  container.addChild(character);
  container.position.set(node.x, node.y);
  container.eventMode = 'static';
  let hovered = false;
  let depth = selected ? 10000 : node.y;
  container.cursor = 'pointer';
  container.hitArea = new pixi.Rectangle(-38 * size, -92 * size, 76 * size, 132 * size);
  container.accessible = false;
  character.addChild(parts.shadow);
  if (parts.selectionRing !== null) character.addChild(parts.selectionRing);
  const body = new pixi.Container();
  body.addChild(options.body);
  if (options.walkingBody !== undefined) {
    options.walkingBody.visible = false;
    body.addChild(options.walkingBody);
  }
  character.addChild(body);
  if (node.status === 'offline') character.alpha = 0.56;
  character.addChild(parts.marker);
  if (parts.bubble !== null) character.addChild(parts.bubble);
  // 名牌与状态贴纸。胶囊宽度按字形估算，与 SVG 回退完全一致；
  // 字体加载完成后场景会整体重建一次，让画布文字换成真正的字体。
  const label = pill(pixi, {
    text: characterLabel(node.displayName),
    fontSize: nameplate.fontSize,
    fontWeight: nameplate.fontWeight,
    paddingX: nameplate.paddingX,
    height: nameplate.height,
    fill: nameplate.fill,
    strokeWidth: nameplate.strokeWidth,
  });
  label.position.set(0, nameplate.top);
  const showLabel =
    selected || (options.showName ?? (node.kind === 'human' || options.detail === 'near'));
  label.visible = showLabel;
  character.addChild(label);
  if (
    showLabel &&
    options.detail === 'near' &&
    options.statusLabel !== undefined &&
    node.kind === 'agent'
  ) {
    const status = pill(pixi, {
      text: options.statusLabel,
      fontSize: statusSticker.fontSize,
      fontWeight: statusSticker.fontWeight,
      paddingX: statusSticker.paddingX,
      height: statusSticker.height,
      fill: statusStickerFill[node.status],
      strokeWidth: statusSticker.strokeWidth,
    });
    status.position.set(0, nameplate.top + nameplate.height + statusSticker.gap);
    character.addChild(status);
  }
  container.on('pointerover', () => {
    hovered = true;
    label.visible = true;
    body.scale.set(1.025);
    options.onInvalidate();
  });
  container.on('pointerout', () => {
    hovered = false;
    label.visible = showLabel;
    body.scale.set(1);
    options.onInvalidate();
  });
  container.on('pointertap', (event: FederatedPointerEvent) => {
    event.stopPropagation();
    options.onSelect(node.characterId);
  });
  return {
    container,
    get depth() {
      return depth;
    },
    animate: (pose) => {
      container.position.set(pose.x, pose.y);
      depth = selected || hovered ? 10000 : pose.y;
      body.position.y = -Math.abs(pose.stride) * 0.42;
      if (options.walkingBody !== undefined) {
        options.walkingBody.visible = pose.moving;
        options.body.visible = !pose.moving;
      }
      body.scale.x = (hovered ? 1.025 : 1) * (pose.moving && pose.facing < 0 ? -1 : 1);
      body.skew.x = pose.facing * (pose.moving ? 0.01 : 0);
    },
    destroy: () => {
      container.destroy({ children: true });
    },
  };
}

type PillOptions = {
  readonly text: string;
  readonly fontSize: number;
  readonly fontWeight: '600';
  readonly paddingX: number;
  readonly height: number;
  readonly fill: string;
  readonly strokeWidth: number;
};

function pill(pixi: PixiModule, options: PillOptions): Container {
  const group = new pixi.Container();
  const text = new pixi.Text({
    text: options.text,
    anchor: { x: 0.5, y: 0.5 },
    style: {
      fill: sceneInk,
      fontFamily: sceneFont,
      fontSize: options.fontSize,
      fontWeight: options.fontWeight,
    },
  });
  const width = estimatedTextWidth(options.text, options.fontSize) + options.paddingX * 2;
  const background = new pixi.Graphics()
    .roundRect(-width / 2, 0, width, options.height, options.height / 2)
    .fill(options.fill)
    .stroke({ color: sceneInk, width: options.strokeWidth });
  text.position.set(0, options.height / 2);
  group.addChild(background, text);
  return group;
}

export function monogram(displayName: string): string {
  const characters = graphemes(displayName.trim()).filter((segment) => segment.trim().length > 0);
  return characters.slice(0, 2).join('').toLocaleUpperCase() || 'AR';
}

function graphemes(value: string): string[] {
  return Array.from(graphemeSegmenter.segment(value), (entry) => entry.segment);
}
