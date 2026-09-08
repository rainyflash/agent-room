import { forwardRef, memo, useImperativeHandle, useRef, useState } from 'react';
import { ChevronDown, LocateFixed, Map } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { roomMapDestination, roomMapViewport } from '../domain/room-map';
import type { LobbySceneProjection, LobbyViewport } from '../domain/scene-projection';
import type { SceneFrame } from '../scene/scene-character';

export type RoomMinimapHandle = { position(frame: SceneFrame): void };

type RoomMinimapProps = {
  readonly projection: LobbySceneProjection;
  readonly onNavigate: (x: number, y: number) => void;
  readonly onHome: () => void;
};

export const RoomMinimap = forwardRef<RoomMinimapHandle, RoomMinimapProps>(function RoomMinimap(
  { projection, onNavigate, onHome },
  ref,
) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(() => window.innerWidth >= 768);
  const boundsRef = useRef<SVGRectElement>(null);
  const viewportRef = useRef<LobbyViewport>({ x: 0, y: 0, ...projection.world, zoom: 1 });
  useImperativeHandle(
    ref,
    () => ({
      position(frame) {
        if (frame.viewport === undefined) return;
        viewportRef.current = frame.viewport;
        const rect = boundsRef.current;
        if (rect === null) return;
        // Camera frames update only the viewport outline; the thousand dots stay mounted.
        const bounds = roomMapViewport(projection.world, frame.viewport);
        for (const key of ['x', 'y', 'width', 'height'] as const)
          rect.setAttribute(key, String(bounds[key]));
      },
    }),
    [projection.world],
  );
  if (projection.nodes.length <= 48) return null;
  const { width, height } = projection.world;
  const navigate = (x: number, y: number): void => {
    const point = roomMapDestination(projection.world, x / width, y / height);
    onNavigate(point.x, point.y);
  };
  return (
    <details
      className="room-minimap"
      open={expanded}
      onToggle={(event) => {
        setExpanded(event.currentTarget.open);
      }}
    >
      <summary>
        <Map size={14} aria-hidden="true" />
        <span>{t('studio.map.label')}</span>
        <ChevronDown size={14} aria-hidden="true" />
      </summary>
      <button
        type="button"
        className="room-minimap__surface"
        aria-label={t('studio.map.navigate')}
        onClick={(event) => {
          if (event.detail === 0) return;
          const box = event.currentTarget.getBoundingClientRect();
          const point = roomMapDestination(
            projection.world,
            (event.clientX - box.left) / box.width,
            (event.clientY - box.top) / box.height,
          );
          onNavigate(point.x, point.y);
        }}
        onKeyDown={(event) => {
          const view = viewportRef.current;
          const x = view.x + view.width / 2;
          const y = view.y + view.height / 2;
          switch (event.key) {
            case 'ArrowLeft':
              navigate(x - view.width * 0.8, y);
              break;
            case 'ArrowRight':
              navigate(x + view.width * 0.8, y);
              break;
            case 'ArrowUp':
              navigate(x, y - view.height * 0.8);
              break;
            case 'ArrowDown':
              navigate(x, y + view.height * 0.8);
              break;
            case 'Home':
              onHome();
              break;
            default:
              return;
          }
          event.preventDefault();
        }}
      >
        <svg aria-hidden="true" viewBox={`0 0 ${String(width)} ${String(height)}`}>
          <rect width={width} height={height} fill="#f8fafb" />
          <MapCharacters projection={projection} />
          <rect
            ref={boundsRef}
            className="room-minimap__viewport"
            x={0}
            y={0}
            width={width}
            height={height}
            fill="#247a7710"
            stroke="#247a77"
            strokeWidth={1.5}
            vectorEffect="non-scaling-stroke"
          />
        </svg>
      </button>
      <footer>
        <span>{t('studio.map.hint')}</span>
        <button
          type="button"
          onClick={onHome}
          aria-label={t('studio.map.home')}
          title={t('studio.map.home')}
        >
          <LocateFixed size={15} aria-hidden="true" />
        </button>
      </footer>
    </details>
  );
});

const MapCharacters = memo(function MapCharacters({
  projection,
}: {
  readonly projection: LobbySceneProjection;
}) {
  const radius = projection.world.width / 180;
  return (
    <g>
      {projection.nodes.map((node) => (
        <circle
          key={node.agentId}
          data-map-agent={node.agentId}
          cx={node.x}
          cy={node.y}
          r={node.agentId === projection.selectedAgentId ? radius * 1.7 : radius}
          fill={node.agentId === projection.selectedAgentId ? '#247a77' : '#9babad'}
        />
      ))}
      {projection.humans?.map((human) => (
        <circle
          key={human.characterId}
          data-map-human={human.characterId}
          cx={human.x}
          cy={human.y}
          r={radius * 2.4}
          fill="#173544"
          stroke="white"
          strokeWidth={radius * 0.8}
        />
      ))}
    </g>
  );
});
