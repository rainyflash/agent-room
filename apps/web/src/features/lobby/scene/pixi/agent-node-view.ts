import type { Container, FederatedPointerEvent } from 'pixi.js';
import type {
  LobbyAgentNodeProjection,
  LobbySceneDetail,
} from '@/features/lobby/domain/scene-projection';
import { characterBodyArt, characterStatusColor } from '../character-art';
import type { CharacterPose } from '../character-motion';
import { shapeGraphics } from './shape-graphics';

type PixiModule = typeof import('pixi.js');
const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
export type AgentNodeViewOptions = {
  readonly detail: LobbySceneDetail;
  readonly node: LobbyAgentNodeProjection;
  readonly onSelect: (agentId: string) => void;
  readonly selected: boolean;
};
export type AgentCharacterView = {
  readonly container: Container;
  animate(pose: CharacterPose): void;
  destroy(): void;
};

export function createAgentNodeView(
  pixi: PixiModule,
  options: AgentNodeViewOptions,
): AgentCharacterView {
  const { node, selected } = options;
  const container = new pixi.Container();
  const size = Math.max(0.83, node.radius / 27);
  const character = new pixi.Container();
  character.scale.set(size);
  container.addChild(character);
  container.position.set(node.x, node.y);
  container.eventMode = 'static';
  container.cursor = 'pointer';
  container.hitArea = new pixi.Rectangle(-28 * size, -82 * size, 56 * size, 98 * size);
  container.accessible = false;
  const shadow = new pixi.Graphics().ellipse(0, 1, 21, 8).fill({ color: '#696c4f', alpha: 0.24 });
  character.addChild(shadow);
  if (selected)
    character.addChild(
      new pixi.Graphics()
        .ellipse(0, 1, 28, 12)
        .stroke({ color: '#fff8da', width: 4 })
        .ellipse(0, 1, 31, 14)
        .stroke({ color: '#769265', width: 2 }),
    );
  const leftLeg = new pixi.Graphics()
    .roundRect(-11, -17, 9, 18, 3)
    .fill('#47564e')
    .roundRect(-13, -4, 12, 6, 2)
    .fill('#eee9d7');
  const rightLeg = new pixi.Graphics()
    .roundRect(2, -17, 9, 18, 3)
    .fill('#47564e')
    .roundRect(2, -4, 12, 6, 2)
    .fill('#eee9d7');
  character.addChild(leftLeg, rightLeg);
  const body = new pixi.Container();
  const arms = new pixi.Graphics()
    .roundRect(-20, -33, 7, 22, 3)
    .fill('#dab493')
    .roundRect(13, -33, 7, 22, 3)
    .fill('#dab493');
  body.addChild(arms, shapeGraphics(pixi, characterBodyArt(node.agentId)));
  character.addChild(body);
  if (node.status === 'offline') character.alpha = 0.56;
  const marker = new pixi.Graphics()
    .circle(19, -61, 6)
    .fill(characterStatusColor[node.status])
    .stroke({ color: '#fff7e2', width: 2 });
  character.addChild(marker);
  if (node.status === 'waiting_input' || node.status === 'blocked') {
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
    const bubble = new pixi.Graphics()
      .roundRect(-10, -92, 23, 23, 8)
      .fill('#fff6d9')
      .stroke({ color: '#ccbb95', width: 1.5 });
    badge.position.set(2, -80);
    character.addChild(bubble, badge);
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
  label.visible = selected || options.detail !== 'distant';
  character.addChild(label);
  container.on('pointerover', () => {
    label.visible = true;
    body.scale.set(1.07);
  });
  container.on('pointerout', () => {
    label.visible = selected || options.detail !== 'distant';
    body.scale.set(1);
  });
  container.on('pointertap', (event: FederatedPointerEvent) => {
    event.stopPropagation();
    options.onSelect(node.agentId);
  });
  return {
    container,
    animate: (pose) => {
      container.position.set(pose.x, pose.y);
      container.zIndex = pose.y;
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
