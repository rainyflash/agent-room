import type { Container, FederatedPointerEvent } from 'pixi.js';

import type { LobbyAgentStatus } from '@/features/lobby/domain/lobby';
import type {
  LobbyAgentNodeProjection,
  LobbySceneDetail,
} from '@/features/lobby/domain/scene-projection';

type PixiModule = typeof import('pixi.js');

const STATUS_COLOR: Readonly<Record<LobbyAgentStatus, number>> = Object.freeze({
  blocked: 0xff_6b_3d,
  completed: 0x9f_e8_70,
  idle: 0x66_c9_d8,
  offline: 0x72_76_71,
  waiting_input: 0xff_6b_3d,
  working: 0x9f_e8_70,
});
const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

export type AgentNodeViewOptions = {
  readonly detail: LobbySceneDetail;
  readonly node: LobbyAgentNodeProjection;
  readonly onSelect: (agentId: string) => void;
  readonly selected: boolean;
};

export function createAgentNodeView(pixi: PixiModule, options: AgentNodeViewOptions): Container {
  const { node } = options;
  const container = new pixi.Container();
  container.position.set(node.x, node.y);
  container.eventMode = 'static';
  container.cursor = 'pointer';
  container.hitArea = new pixi.Circle(0, 0, node.radius + 10);
  // 可访问语义由 React DOM 映射统一提供，避免 Pixi Overlay 生成第二套重复焦点树。
  container.accessible = false;

  if (options.selected) {
    container.addChild(
      new pixi.Graphics()
        .circle(0, 0, node.radius + 9)
        .stroke({ alpha: 0.78, color: 0xf2_f0_e9, width: 2 }),
    );
  }
  container.addChild(
    new pixi.Graphics()
      .circle(0, 0, node.radius + 3)
      .stroke({
        alpha: node.status === 'offline' ? 0.42 : 0.96,
        color: STATUS_COLOR[node.status],
        width: 3,
      })
      .circle(0, 0, node.radius)
      .fill({ color: 0x1a_1d_19 }),
  );
  if (options.detail !== 'distant') {
    container.addChild(
      new pixi.Text({
        anchor: 0.5,
        style: {
          fill: 0xf2_f0_e9,
          fontFamily: 'Instrument Sans, sans-serif',
          fontSize: Math.max(12, node.radius * 0.62),
          fontWeight: '600',
        },
        text: monogram(node.displayName),
      }),
    );
  }
  if (options.detail === 'near') {
    const label = new pixi.Text({
      anchor: { x: 0.5, y: 0 },
      style: {
        fill: 0xf2_f0_e9,
        fontFamily: 'Instrument Sans, sans-serif',
        fontSize: 14,
        fontWeight: '500',
      },
      text: truncateLabel(node.displayName),
    });
    label.position.set(0, node.radius + 12);
    container.addChild(label);
  }
  container.on('pointertap', (event: FederatedPointerEvent) => {
    event.stopPropagation();
    options.onSelect(node.agentId);
  });
  return container;
}

export function monogram(displayName: string): string {
  const characters = graphemes(displayName.trim()).filter((segment) => segment.trim().length > 0);
  return characters.slice(0, 2).join('').toLocaleUpperCase() || 'AR';
}

function truncateLabel(displayName: string): string {
  const characters = graphemes(displayName.trim());
  return characters.length <= 18 ? displayName.trim() : `${characters.slice(0, 17).join('')}…`;
}

function graphemes(value: string): string[] {
  return Array.from(graphemeSegmenter.segment(value), (entry) => entry.segment);
}
