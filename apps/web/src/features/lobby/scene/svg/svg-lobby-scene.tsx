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
import { visibleLobbyNodes, type LobbySceneProjection } from '../../domain/scene-projection';
import { ViewportController, type CameraSnapshot } from '../viewport-controller';
import { characterStatusColor } from '../character-art';
import { RoomPlan } from './room-plan';
import { roomCrowdGroups, usesCrowdOverview } from '../../domain/room-crowd';
import { StudioSprite } from './studio-sprite';

export type SvgLobbySceneHandle = {
  resetViewport(): void;
  focusAgent(agentId: string): void;
  focusArea(x: number, y: number): void;
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
      minimumScale: 0.04,
      ...(projection.nodes.length <= 48 ? { compactInitialScale: 0.48 } : {}),
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
        focusArea: (x, y) => {
          if (controllerRef.current !== null) commitCamera(controllerRef.current.focusArea(x, y));
        },
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
      const controller = controllerRef.current;
      if (controller === null) return;
      controller.updateWorld(projection.world);
      commitCamera(controller.snapshot());
    }, [projection.world.width, projection.world.height, commitCamera]);

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
    const worldViewport = {
      x: -camera.x / camera.scale,
      y: -camera.y / camera.scale,
      width: viewport.width / camera.scale,
      height: viewport.height / camera.scale,
      zoom: camera.scale,
    };
    const overview = usesCrowdOverview(projection.nodes.length, camera.scale);
    const visibleIds = new Set(
      (overview
        ? []
        : visibleLobbyNodes(projection, {
            ...worldViewport,
            x: worldViewport.x - 100,
            y: worldViewport.y - 100,
            width: worldViewport.width + 200,
            height: worldViewport.height + 200,
          })
      ).map((node) => node.agentId),
    );
    const characters = sceneCharacters(projection, labels.self).filter(
      (node) => node.kind === 'human' || visibleIds.has(node.characterId),
    );
    useEffect(() => {
      onFrame?.({
        ...viewport,
        overview,
        groups: roomCrowdGroups(projection, worldViewport).map((group) => ({
          ...group,
          screenX: camera.x + group.x * camera.scale,
          screenY: camera.y + group.y * camera.scale,
        })),
        characters: overview
          ? []
          : characters.map((node) => ({
              characterId: node.characterId,
              x: camera.x + node.x * camera.scale,
              y: camera.y + (node.y - 100 * Math.max(0.83, node.radius / 27)) * camera.scale,
            })),
      });
    }, [camera, projection, viewport, onFrame, labels.self]);
    const objects = [
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
              showName={
                node.kind === 'human' || projection.nodes.length <= 24 || camera.scale >= 1.18
              }
              statusLabel={
                projection.nodes.length <= 24 || camera.scale >= 1.18
                  ? labels.statuses?.[node.status]
                  : undefined
              }
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
        data-agent-room-overview={overview}
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
          <RoomPlan world={projection.world} />
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
  statusLabel,
}: {
  readonly node: SceneCharacter;
  readonly selected: boolean;
  readonly showName: boolean;
  readonly statusLabel: string | undefined;
}) {
  return (
    <>
      <ellipse cx="0" cy="1" rx="21" ry="8" fill="#696c4f" opacity="0.24" />
      {selected ? (
        <ellipse cx="0" cy="1" rx="28" ry="12" fill="none" stroke="#fff8da" strokeWidth="4" />
      ) : null}
      <g opacity={node.status === 'offline' ? 0.56 : 1}>
        {node.kind === 'human' ? (
          <g>
            <polygon points="-28,-32 -14,-57 14,-57 28,-32 14,-7 -14,-7" fill="#173544" />
            <circle cx="0" cy="-32" r="10" fill="white" />
          </g>
        ) : (
          <StudioSprite id={node.characterId} />
        )}
        <circle
          cx="30"
          cy="-70"
          r="4"
          fill={characterStatusColor[node.status]}
          stroke="#fff7e2"
          strokeWidth="2"
        />
      </g>
      {node.status === 'waiting_input' || node.status === 'blocked' ? (
        <g>
          <rect x="25" y="-106" width="23" height="23" rx="8" fill="#fff6d9" stroke="#ccbb95" />
          <text
            x="37"
            y="-90"
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
        y="30"
        data-visible={selected || showName}
      >
        {node.displayName}
      </text>
      {statusLabel === undefined || node.kind === 'human' ? null : (
        <text x="0" y="50" textAnchor="middle" className="room-character-status">
          {statusLabel}
        </text>
      )}
      <rect x="-38" y="-92" width="76" height="132" fill="transparent" />
    </>
  );
}
