import type { SceneCharacter } from '../scene-character';
import type { Container, FederatedPointerEvent, Sprite } from 'pixi.js';
import type { LobbySceneDetail } from '@/features/lobby/domain/scene-projection';
import type { CharacterPose } from '../character-motion';
import type { CharacterParts } from './character-texture-cache';

type PixiModule = typeof import('pixi.js');
const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
export type AgentNodeViewOptions = {
  readonly body: Sprite;
  readonly parts: CharacterParts;
  readonly detail: LobbySceneDetail;
  readonly node: SceneCharacter;
  readonly onInvalidate: () => void;
  readonly onSelect: (agentId: string) => void;
  readonly selected: boolean;
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
  container.hitArea = new pixi.Rectangle(-28 * size, -82 * size, 56 * size, 98 * size);
  container.accessible = false;
  character.addChild(parts.shadow);
  if (parts.selectionRing !== null) character.addChild(parts.selectionRing);
  const { leftLeg, rightLeg, arms } = parts;
  character.addChild(leftLeg, rightLeg);
  const body = new pixi.Container();
  body.addChild(arms, options.body);
  character.addChild(body);
  if (node.status === 'offline') character.alpha = 0.56;
  character.addChild(parts.marker);
  if (parts.bubble !== null) {
    const badge = new pixi.Text({
      text: node.status === 'blocked' ? '!' : '?',
      anchor: 0.5,
      style: {
        fill: '#74502e',
        fontFamily: 'Instrument Sans, sans-serif',
        fontSize: 19,
        fontWeight: '700',
      },
    });
    badge.position.set(2, -80);
    character.addChild(parts.bubble, badge);
  }
  const label = new pixi.Text({
    text: truncateLabel(node.displayName),
    anchor: { x: 0.5, y: 0 },
    style: {
      fill: '#354b3e',
      fontFamily: 'Instrument Sans, Noto Sans SC, sans-serif',
      fontSize: 18,
      fontWeight: '600',
      stroke: { color: '#f7f3df', width: 3 },
    },
  });
  label.position.set(0, 14);
  label.visible = selected || node.kind === 'human' || options.detail !== 'distant';
  character.addChild(label);
  container.on('pointerover', () => {
    hovered = true;
    label.visible = true;
    body.scale.set(1.07);
    options.onInvalidate();
  });
  container.on('pointerout', () => {
    hovered = false;
    label.visible = selected || node.kind === 'human' || options.detail !== 'distant';
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
      leftLeg.position.y = pose.stride;
      rightLeg.position.y = -pose.stride;
      arms.rotation = pose.stride * 0.015;
      body.skew.x = pose.facing * (pose.moving ? 0.045 : 0);
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
