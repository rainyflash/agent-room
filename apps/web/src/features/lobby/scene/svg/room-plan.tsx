import { roomPlanShapes } from '../room-plan-art';
import type { LobbyWorld } from '../../domain/scene-projection';

export function RoomPlan({ world }: { readonly world: LobbyWorld }) {
  return (
    <g data-room-art="flat-plan" strokeWidth="2">
      {roomPlanShapes(world).map((shape, index) => {
        if (shape.kind === 'line')
          return (
            <line
              key={index}
              x1={shape.x}
              y1={shape.y}
              x2={shape.toX}
              y2={shape.toY}
              stroke={shape.stroke}
            />
          );
        if (shape.kind === 'ellipse')
          return (
            <ellipse
              key={index}
              cx={shape.x}
              cy={shape.y}
              rx={shape.rx}
              ry={shape.ry}
              fill={shape.fill}
              stroke={shape.stroke}
            />
          );
        return (
          <rect
            key={index}
            x={shape.x}
            y={shape.y}
            width={shape.width}
            height={shape.height}
            rx={shape.radius}
            fill={shape.fill}
            stroke={shape.stroke}
          />
        );
      })}
    </g>
  );
}
