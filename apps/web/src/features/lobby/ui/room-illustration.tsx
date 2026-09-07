import { memo } from 'react';
import { projectFloorPoint } from '@/features/lobby/domain/room-floor';
import { characterStillArt } from '@/features/lobby/scene/character-art';
import { roomGroundArt, roomPropsArt } from '@/features/lobby/scene/room-art';
import { SceneShapes } from '@/features/lobby/scene/svg/scene-shapes';

// Decorative room art shares the actual scene geometry. These figures never enter live presence.
const ground = roomGroundArt();
const objects = [
  ...roomPropsArt().map((prop, index) => ({
    key: `furniture-${String(index)}`,
    depth: prop.depth,
    element: <SceneShapes shapes={prop.shapes} />,
  })),
  ...[
    { x: 185, y: 290 },
    { x: 470, y: 320 },
    { x: 800, y: 300 },
    { x: 1140, y: 680 },
    { x: 1400, y: 670 },
    { x: 1250, y: 270 },
    { x: 320, y: 590 },
    { x: 700, y: 750 },
    { x: 1010, y: 900 },
    { x: 260, y: 930 },
    { x: 1460, y: 880 },
    { x: 450, y: 90 },
  ].map((position, index) => {
    const point = projectFloorPoint(position);
    return {
      key: `illustration-${String(index)}`,
      depth: point.y,
      element: (
        <g transform={`translate(${String(point.x)} ${String(point.y)}) scale(1.3)`}>
          <ellipse cx="0" cy="2" rx="23" ry="8" fill="#62775333" />
          <SceneShapes shapes={characterStillArt(`illustration-${String(index)}`)} />
        </g>
      ),
    };
  }),
].sort((left, right) => left.depth - right.depth);

export const RoomIllustration = memo(function RoomIllustration() {
  return (
    <svg
      aria-hidden="true"
      className="room-illustration"
      focusable="false"
      viewBox="235 10 2340 1320"
    >
      <SceneShapes shapes={ground} />
      {objects.map((object) => (
        <g key={object.key}>{object.element}</g>
      ))}
    </svg>
  );
});

export function AgentPortrait({ id }: { readonly id: string }) {
  return (
    <svg aria-hidden="true" className="agent-portrait" focusable="false" viewBox="-30 -75 60 85">
      <ellipse cx="0" cy="2" rx="22" ry="6" fill="#62775326" />
      <SceneShapes shapes={characterStillArt(id)} />
    </svg>
  );
}
