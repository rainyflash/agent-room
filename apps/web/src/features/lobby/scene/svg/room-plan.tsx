import { useId } from 'react';

import { roomPlanShapes } from '../room-plan-art';
import { sceneFloor } from '../scene-style';
import type { LobbyWorld } from '../../domain/scene-projection';

export function RoomPlan({ world }: { readonly world: LobbyWorld }) {
  // 同一页面可能同时有好几张房间插图，图案 id 必须各不相同。
  const floorPattern = `room-floor-${useId().replaceAll(':', '')}`;
  const tile = sceneFloor.tile;
  return (
    <g data-room-art="flat-plan">
      <defs>
        <pattern id={floorPattern} width={tile * 2} height={tile * 2} patternUnits="userSpaceOnUse">
          <rect width={tile * 2} height={tile * 2} fill={sceneFloor.light} />
          <rect width={tile} height={tile} fill={sceneFloor.dark} />
          <rect x={tile} y={tile} width={tile} height={tile} fill={sceneFloor.dark} />
        </pattern>
      </defs>
      {roomPlanShapes(world).map((shape, index) => {
        if (shape.kind === 'floor')
          return (
            <rect
              key={index}
              x={shape.x}
              y={shape.y}
              width={shape.width}
              height={shape.height}
              fill={`url(#${floorPattern})`}
            />
          );
        if (shape.kind === 'line')
          return (
            <line
              key={index}
              x1={shape.x}
              y1={shape.y}
              x2={shape.toX}
              y2={shape.toY}
              stroke={shape.stroke}
              strokeWidth={shape.strokeWidth}
              strokeLinecap="round"
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
              strokeWidth={shape.strokeWidth}
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
            strokeWidth={shape.strokeWidth}
          />
        );
      })}
    </g>
  );
}
