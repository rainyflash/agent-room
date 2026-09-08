import type { SceneCharacter } from '../scene-character';
import type { Container, FederatedPointerEvent, Sprite } from 'pixi.js';
import type { LobbySceneDetail } from '@/features/lobby/domain/scene-projection';
import type { CharacterPose } from '../character-motion';
import type { CharacterParts } from './character-texture-cache';

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
  const label = new pixi.Text({
    text: truncateLabel(node.displayName),
    anchor: { x: 0.5, y: 0 },
    style: {
      fill: '#233039',
      fontFamily: 'Instrument Sans Variable, Noto Sans SC Variable, sans-serif',
      fontSize: 19,
      fontWeight: '600',
      stroke: { color: '#fff', width: 4 },
    },
  });
  label.position.set(0, 14);
  label.visible = selected || node.kind === 'human' || options.detail === 'near';
  character.addChild(label);
  if (options.detail === 'near' && options.statusLabel !== undefined && node.kind === 'agent') {
    const status = new pixi.Text({
      text: options.statusLabel,
      anchor: { x: 0.5, y: 0 },
      style: {
        fill: '#526667',
        fontFamily: 'Instrument Sans Variable, Noto Sans SC Variable, sans-serif',
        fontSize: 13,
        stroke: { color: '#fff', width: 3 },
      },
    });
    status.position.set(0, 38);
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
    label.visible = selected || node.kind === 'human' || options.detail === 'near';
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

export function monogram(displayName: string): string {
  const characters = graphemes(displayName.trim()).filter((segment) => segment.trim().length > 0);
  return characters.slice(0, 2).join('').toLocaleUpperCase() || 'AR';
}

function truncateLabel(displayName: string): string {
  const characters = graphemes(displayName.trim());
  return characters.length <= 20 ? displayName.trim() : `${characters.slice(0, 19).join('')}…`;
}

function graphemes(value: string): string[] {
  return Array.from(graphemeSegmenter.segment(value), (entry) => entry.segment);
}
