import type { FloorPoint } from './room-floor';
import type {
  LobbyBounds,
  LobbySceneProjection,
  LobbyViewport,
  LobbyWorld,
} from './scene-projection';

/** Start by the human; positions never imply a team or a project. */
export function roomHome(scene: LobbySceneProjection): FloorPoint {
  const self = scene.humans?.find((human) => human.isSelf);
  return self === undefined
    ? { x: scene.world.width / 2, y: scene.world.height / 2 }
    : { x: self.x, y: self.y };
}

export function roomMapDestination(
  world: LobbyWorld,
  fractionX: number,
  fractionY: number,
): FloorPoint {
  const fraction = (value: number): number =>
    Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0.5;
  return { x: world.width * fraction(fractionX), y: world.height * fraction(fractionY) };
}

/** Focused edge characters may leave camera space beyond the room; the map shows the intersection. */
export function roomMapViewport(world: LobbyWorld, viewport: LobbyViewport): LobbyBounds {
  const x = Math.max(0, Math.min(world.width, viewport.x));
  const y = Math.max(0, Math.min(world.height, viewport.y));
  return {
    x,
    y,
    width: Math.max(0, Math.min(world.width, viewport.x + viewport.width) - x),
    height: Math.max(0, Math.min(world.height, viewport.y + viewport.height) - y),
  };
}
