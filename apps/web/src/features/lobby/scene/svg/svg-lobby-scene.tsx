import { sceneCharacters, type SceneCharacter, type SceneFrame } from '../scene-character';
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from 'react';
import type { LobbySceneLabels } from '../lobby-scene';
import type { LobbySceneProjection } from '../../domain/scene-projection';
import { ViewportController, type CameraSnapshot } from '../viewport-controller';
import { characterBodyArt, characterStatusColor } from '../character-art';
import { roomGroundArt, roomPlaques, roomPropsArt } from '../room-art';
import { SceneShapes } from './scene-shapes';

export type SvgLobbySceneHandle = {
  resetViewport(): void;
  focusAgent(agentId: string): void;
  zoomBy(factor: number): void;
};
export type SvgLobbySceneProps = {
  readonly onFrame?: (frame: SceneFrame) => void;
  readonly onSelectHuman?: (matrixUserId: string) => void;
  readonly labels: LobbySceneLabels;
  readonly onSelectAgent: (agentId: string | null) => void;
  readonly onZoomChange: (zoom: number) => void;
  readonly projection: LobbySceneProjection;
};
type Point = { readonly x: number; readonly y: number };
const ground = roomGroundArt();
const furniture = roomPropsArt();

export const SvgLobbyScene = forwardRef<SvgLobbySceneHandle, SvgLobbySceneProps>(
  function SvgLobbyScene(
    { labels, onSelectAgent, onZoomChange, onFrame, onSelectHuman, projection },
    forwardedRef,
  ) {
    const hostRef = useRef<SVGSVGElement>(null);
    const controllerRef = useRef<ViewportController | null>(null);
    const pointers = useRef(new Map<number, Point>());
    const gestureMoved = useRef(false);
    const [camera, setCamera] = useState<CameraSnapshot>({ scale: 1, x: 0, y: 0 });
    const [viewport, setViewport] = useState({ height: 1, width: 1 });
    controllerRef.current ??= new ViewportController(projection.world, {
      padding: 22,
      minimumScale: 0.22,
    });

    const commitCamera = useCallback(
      (next: CameraSnapshot): void => {
        setCamera(next);
        onZoomChange(next.scale);
      },
      [onZoomChange],
    );

    useImperativeHandle(
      forwardedRef,
      () => ({
        focusAgent: (id) => {
          const node = projection.nodes.find((candidate) => candidate.agentId === id);
          if (node !== undefined && controllerRef.current !== null)
            commitCamera(controllerRef.current.focusOn(node.x, node.y - 35));
        },
        resetViewport: () => {
          if (controllerRef.current !== null) commitCamera(controllerRef.current.reset());
        },
        zoomBy: (factor) => {
          if (controllerRef.current !== null) commitCamera(controllerRef.current.zoomBy(factor));
        },
      }),
      [commitCamera, projection.nodes],
    );

    useEffect(() => {
      const host = hostRef.current;
      const controller = controllerRef.current;
      if (host === null || controller === null) return undefined;
      const resize = (): void => {
        const bounds = host.getBoundingClientRect();
        const width = Math.max(1, bounds.width);
        const height = Math.max(1, bounds.height);
        setViewport({ height, width });
        commitCamera(controller.resize(width, height));
      };
      resize();
      const observer = new ResizeObserver(resize);
      observer.observe(host);
      return () => {
        observer.disconnect();
      };
    }, [commitCamera]);

    const pointerPosition = (
      event: ReactPointerEvent<SVGSVGElement> | ReactWheelEvent<SVGSVGElement>,
    ): Point => {
      const bounds = event.currentTarget.getBoundingClientRect();
      return { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
    };
    const select = (agentId: string | null): void => {
      if (!gestureMoved.current) onSelectAgent(agentId);
      gestureMoved.current = false;
    };
    const characters = sceneCharacters(projection, labels.self);
    useEffect(() => {
      onFrame?.({
        ...viewport,
        characters: characters.map((node) => ({
          characterId: node.characterId,
          x: camera.x + node.x * camera.scale,
          y: camera.y + (node.y - 95 * Math.max(0.83, node.radius / 27)) * camera.scale,
        })),
      });
    }, [camera, projection, viewport, onFrame, labels.self]);
    const objects = [
      ...furniture.map((prop, index) => ({
        key: `prop-${String(index)}`,
        depth: prop.depth,
        element: <SceneShapes shapes={prop.shapes} />,
      })),
      ...characters.map((node) => ({
        key: node.characterId,
        depth: node.characterId === projection.selectedAgentId ? 10000 : node.y,
        element: (
          <g
            className="lobby-scene__svg-agent"
            data-character-id={node.characterId}
            data-selected={node.characterId === projection.selectedAgentId}
            onClick={(event) => {
              event.stopPropagation();
              if (node.kind === 'human') {
                if (!gestureMoved.current) onSelectHuman?.(node.matrixUserId);
                gestureMoved.current = false;
              } else select(node.characterId);
            }}
            transform={`translate(${String(node.x)} ${String(node.y)}) scale(${String(Math.max(0.83, node.radius / 27))})`}
          >
            <SvgCharacter
              node={node}
              selected={node.characterId === projection.selectedAgentId}
              showName={node.kind === 'human' || camera.scale >= 0.68}
            />
          </g>
        ),
      })),
    ].sort((a, b) => a.depth - b.depth);

    return (
      <svg
        aria-hidden="true"
        className="lobby-scene__svg"
        data-renderer="svg"
        ref={hostRef}
        viewBox={`0 0 ${String(viewport.width)} ${String(viewport.height)}`}
        onClick={() => {
          select(null);
        }}
        onPointerDown={(event) => {
          if (pointers.current.size === 0) gestureMoved.current = false;
          pointers.current.set(event.pointerId, pointerPosition(event));
        }}
        onPointerMove={(event) => {
          const previous = pointers.current.get(event.pointerId);
          const controller = controllerRef.current;
          if (previous === undefined || controller === null) return;
          const next = pointerPosition(event);
          const other = [...pointers.current.entries()].find(([id]) => id !== event.pointerId)?.[1];
          if (Math.abs(next.x - previous.x) + Math.abs(next.y - previous.y) > 2) {
            gestureMoved.current = true;
            event.currentTarget.setPointerCapture(event.pointerId);
          }
          pointers.current.set(event.pointerId, next);
          if (other === undefined) {
            commitCamera(controller.panBy(next.x - previous.x, next.y - previous.y));
          } else {
            gestureMoved.current = true;
            const oldDistance = Math.hypot(previous.x - other.x, previous.y - other.y);
            const distance = Math.hypot(next.x - other.x, next.y - other.y);
            if (oldDistance > 0)
              commitCamera(
                controller.zoomBy(
                  distance / oldDistance,
                  (next.x + other.x) / 2,
                  (next.y + other.y) / 2,
                ),
              );
          }
        }}
        onPointerUp={(event) => {
          pointers.current.delete(event.pointerId);
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={(event) => {
          pointers.current.delete(event.pointerId);
          gestureMoved.current = true;
        }}
        onWheel={(event) => {
          event.preventDefault();
          if (controllerRef.current === null) return;
          const point = pointerPosition(event);
          commitCamera(
            controllerRef.current.zoomBy(Math.exp(-event.deltaY * 0.0012), point.x, point.y),
          );
        }}
      >
        <g
          transform={`translate(${String(camera.x)} ${String(camera.y)}) scale(${String(camera.scale)})`}
        >
          <SceneShapes shapes={ground} />
          {roomPlaques.map((plaque) => (
            <text
              className="room-floor-label"
              key={plaque.id}
              x={plaque.x}
              y={plaque.y}
              textAnchor="middle"
            >
              {labels.zones[plaque.id]}
            </text>
          ))}
          {objects.map((object) => (
            <g key={object.key}>{object.element}</g>
          ))}
        </g>
      </svg>
    );
  },
);

function SvgCharacter({
  node,
  selected,
  showName,
}: {
  readonly node: SceneCharacter;
  readonly selected: boolean;
  readonly showName: boolean;
}) {
  return (
    <>
      <ellipse cx="0" cy="1" rx="21" ry="8" fill="#696c4f" opacity="0.24" />
      {selected ? (
        <ellipse cx="0" cy="1" rx="28" ry="12" fill="none" stroke="#fff8da" strokeWidth="4" />
      ) : null}
      <g opacity={node.status === 'offline' ? 0.56 : 1}>
        <rect x="-11" y="-17" width="9" height="18" rx="3" fill="#47564e" />
        <rect x="2" y="-17" width="9" height="18" rx="3" fill="#47564e" />
        <rect x="-13" y="-4" width="12" height="6" rx="2" fill="#eee9d7" />
        <rect x="2" y="-4" width="12" height="6" rx="2" fill="#eee9d7" />
        <rect x="-20" y="-33" width="7" height="22" rx="3" fill="#dab493" />
        <rect x="13" y="-33" width="7" height="22" rx="3" fill="#dab493" />
        <SceneShapes shapes={characterBodyArt(node.characterId, node.kind)} />
        <circle
          cx="19"
          cy="-61"
          r="6"
          fill={characterStatusColor[node.status]}
          stroke="#fff7e2"
          strokeWidth="2"
        />
      </g>
      {node.status === 'waiting_input' || node.status === 'blocked' ? (
        <g>
          <rect x="-10" y="-92" width="23" height="23" rx="8" fill="#fff6d9" stroke="#ccbb95" />
          <text
            x="2"
            y="-76"
            textAnchor="middle"
            style={{ fill: '#74502e', fontSize: 19, fontWeight: 700 }}
          >
            {node.status === 'blocked' ? '!' : '?'}
          </text>
        </g>
      ) : null}
      <text
        className="room-character-name"
        textAnchor="middle"
        y="32"
        data-visible={selected || showName}
      >
        {node.displayName}
      </text>
      <rect x="-28" y="-82" width="56" height="98" fill="transparent" />
    </>
  );
}
