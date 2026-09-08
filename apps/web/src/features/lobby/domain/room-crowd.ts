import type {
  LobbyAgentNodeProjection,
  LobbySceneProjection,
  LobbyViewport,
} from './scene-projection';

export type RoomCrowdGroup = {
  readonly id: string;
  readonly count: number;
  readonly attention: number;
  readonly working: number;
  readonly x: number;
  readonly y: number;
};

export function usesCrowdOverview(count: number, zoom: number): boolean {
  return count > 48 && zoom < 0.62;
}

/** Aggregate stable world cells, then cull; panning must not change a group's count. */
export function roomCrowdGroups(
  scene: LobbySceneProjection,
  viewport: LobbyViewport,
): readonly RoomCrowdGroup[] {
  if (!usesCrowdOverview(scene.nodes.length, viewport.zoom)) return [];
  const targetPixels = viewport.width * viewport.zoom < 768 ? 112 : 160;
  const size = 128 * 2 ** Math.max(0, Math.ceil(Math.log2(targetPixels / (128 * viewport.zoom))));
  const cells = new Map<string, LobbyAgentNodeProjection[]>();
  for (const node of scene.nodes) {
    const x = Math.floor(node.x / size);
    const y = Math.floor(node.y / size);
    const id = `${String(size)}:${String(x)}:${String(y)}`;
    let cell = cells.get(id);
    if (cell === undefined) {
      cell = [];
      cells.set(id, cell);
    }
    cell.push(node);
  }
  return [...cells].flatMap(([id, cell]) => {
    // A cell's true bounds can straddle the viewport. Centering within member bounds also
    // keeps the last groups inside the room instead of beyond its walls.
    const x = cell.reduce((sum, node) => sum + node.x, 0) / cell.length;
    const y = cell.reduce((sum, node) => sum + node.y, 0) / cell.length;
    const margin = 42 / viewport.zoom;
    if (
      x < viewport.x - margin ||
      x > viewport.x + viewport.width + margin ||
      y < viewport.y - margin ||
      y > viewport.y + viewport.height + margin
    )
      return [];
    return [
      {
        id,
        x,
        y,
        count: cell.length,
        attention: cell.filter(
          (node) => node.status === 'waiting_input' || node.status === 'blocked',
        ).length,
        working: cell.filter((node) => node.status === 'working').length,
      },
    ];
  });
}
