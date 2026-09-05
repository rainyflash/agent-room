import type { SceneCharacter } from './scene-character';
import {
  characterSeed,
  isWalkableFloor,
  projectFloorPoint,
} from '@/features/lobby/domain/room-floor';

export type CharacterPose = {
  readonly x: number;
  readonly y: number;
  readonly stride: number;
  readonly facing: number;
  readonly moving: boolean;
};

export function characterPose(
  node: SceneCharacter,
  elapsedSeconds: number,
  movingAllowed: boolean,
): CharacterPose {
  const still = { x: node.x, y: node.y, stride: 0, facing: 1, moving: false };
  if (
    !movingAllowed ||
    node.floorPosition === undefined ||
    node.status === 'offline' ||
    node.status === 'present'
  )
    return still;
  const seed = characterSeed(node.characterId);
  const time = elapsedSeconds + (seed % 1300) / 100;
  if (node.status !== 'idle' && node.status !== 'completed') {
    return { ...still, stride: Math.sin(time * 2) * 0.7 };
  }
  const angle = (seed % 628) / 100;
  const distance = Math.min(34 + (seed % 36), node.roamingRadius ?? 69);
  if (distance < 3) return { ...still, stride: Math.sin(time * 2) * 0.5 };
  const target = {
    x: node.floorPosition.x + Math.cos(angle) * distance,
    y: node.floorPosition.y + Math.sin(angle) * distance,
  };
  for (let step = 1; step <= 8; step += 1) {
    if (
      !isWalkableFloor({
        x: node.floorPosition.x + ((target.x - node.floorPosition.x) * step) / 8,
        y: node.floorPosition.y + ((target.y - node.floorPosition.y) * step) / 8,
      })
    )
      return { ...still, stride: Math.sin(time * 2) * 0.7 };
  }
  const cycle = time % 18;
  const progress = cycle < 5 ? cycle / 5 : cycle < 9 ? 1 : cycle < 14 ? 1 - (cycle - 9) / 5 : 0;
  const moving = cycle < 5 || (cycle >= 9 && cycle < 14);
  const point = projectFloorPoint({
    x: node.floorPosition.x + (target.x - node.floorPosition.x) * progress,
    y: node.floorPosition.y + (target.y - node.floorPosition.y) * progress,
  });
  return {
    ...point,
    moving,
    stride: moving ? Math.sin(time * 9) * 3.5 : 0,
    facing: (cycle < 9 ? 1 : -1) * (Math.cos(angle) - Math.sin(angle) >= 0 ? 1 : -1),
  };
}
