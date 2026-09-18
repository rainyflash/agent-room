import { visibleCharacterLabels, characterLabel } from '../character-labels';
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
import {
  sceneDetailForRoom,
  visibleLobbyNodes,
  type LobbySceneProjection,
} from '../../domain/scene-projection';
import { ViewportController, type CameraSnapshot } from '../viewport-controller';
import {
  characterShadow,
  estimatedTextWidth,
  humanMarker,
  nameplate,
  sceneInk,
  selectionRing,
  statusBadgeFill,
  statusSticker,
  statusStickerFill,
} from '../scene-style';
import { RoomPlan } from './room-plan';
import { roomHome } from '../../domain/room-map';
import { StudioSprite } from './studio-sprite';

export type SvgLobbySceneHandle = {
  releaseFocus(): void;
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
    const initialized = useRef(false);
    const projectionRef = useRef(projection);
    projectionRef.current = projection;
    const [camera, setCamera] = useState<CameraSnapshot>({ scale: 1, x: 0, y: 0 });
    const [viewport, setViewport] = useState({ height: 1, width: 1 });
    const [hovered, setHovered] = useState<string | null>(null);
    controllerRef.current ??= new ViewportController(projection.world, {
      padding: 22,
      minimumScale: 0.3,
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
        releaseFocus: () => {
          if (controllerRef.current !== null) commitCamera(controllerRef.current.releaseFocus());
        },
        focusArea: (x, y) => {
          if (controllerRef.current !== null) commitCamera(controllerRef.current.focusArea(x, y));
        },
        focusAgent: (id) => {
          const node = projection.nodes.find((candidate) => candidate.agentId === id);
          if (node !== undefined && controllerRef.current !== null)
            commitCamera(controllerRef.current.focusOn(node.x, node.y - 35));
        },
        resetViewport: () => {
          const controller = controllerRef.current;
          if (controller === null) return;
          const home = roomHome(projection);
          controller.reset();
          commitCamera(
            projection.nodes.length > 48
              ? controller.focusArea(home.x, home.y)
              : controller.reset(),
          );
        },
        zoomBy: (factor) => {
          if (controllerRef.current !== null) commitCamera(controllerRef.current.zoomBy(factor));
        },
      }),
      [commitCamera, projection],
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
        controller.resize(width, height);
        if (!initialized.current && projectionRef.current.nodes.length > 48) {
          const home = roomHome(projectionRef.current);
          controller.focusArea(home.x, home.y);
        }
        initialized.current = true;
        commitCamera(controller.snapshot());
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
    const visibleIds = new Set(
      visibleLobbyNodes(projection, {
        ...worldViewport,
        x: worldViewport.x - 100,
        y: worldViewport.y - 100,
        width: worldViewport.width + 200,
        height: worldViewport.height + 200,
      }).map((node) => node.agentId),
    );
    const characters = sceneCharacters(projection, labels.self).filter(
      (node) => node.kind === 'human' || visibleIds.has(node.characterId),
    );
    useEffect(() => {
      onFrame?.({
        ...viewport,
        viewport: worldViewport,
        characters: characters.map((node) => ({
          characterId: node.characterId,
          x: camera.x + node.x * camera.scale,
          y: camera.y + (node.y - 100 * Math.max(0.83, node.radius / 27)) * camera.scale,
        })),
      });
    }, [camera, projection, viewport, onFrame, labels.self]);
    const near = sceneDetailForRoom(projection.nodes.length, camera.scale) === 'near';
    const names = visibleCharacterLabels(characters, projection.selectedAgentId, near, (node) =>
      characterStatusLabel(node, labels),
    );
    // 人物按深度排序后，名牌与状态贴纸在第二层整体画在所有人物之上，不会被站在下方的人物挡住。
    const ordered = characters
      .map((node) => ({
        node,
        selected: node.characterId === projection.selectedAgentId,
        transform: `translate(${String(node.x)} ${String(node.y)}) scale(${String(Math.max(0.83, node.radius / 27))})`,
      }))
      .sort(
        (a, b) =>
          (a.selected || a.node.characterId === hovered ? 10000 : a.node.y) -
          (b.selected || b.node.characterId === hovered ? 10000 : b.node.y),
      );

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
          <RoomPlan world={projection.world} />
          {ordered.map(({ node, selected, transform }) => (
            <g
              className="lobby-scene__svg-agent"
              data-character-id={node.characterId}
              data-selected={selected}
              key={node.characterId}
              onClick={(event) => {
                event.stopPropagation();
                if (node.kind === 'human') {
                  if (!gestureMoved.current) onSelectHuman?.(node.matrixUserId);
                  gestureMoved.current = false;
                } else select(node.characterId);
              }}
              onPointerEnter={() => {
                setHovered(node.characterId);
              }}
              onPointerLeave={() => {
                setHovered((current) => (current === node.characterId ? null : current));
              }}
              transform={transform}
            >
              <SvgCharacter node={node} selected={selected} />
            </g>
          ))}
          <g className="lobby-scene__svg-labels" pointerEvents="none">
            {ordered.map(({ node, selected, transform }) => (
              <g
                className="lobby-scene__svg-label"
                data-label-for={node.characterId}
                data-selected={selected}
                key={node.characterId}
                transform={transform}
              >
                <SvgCharacterLabels
                  node={node}
                  visible={selected || names.has(node.characterId) || hovered === node.characterId}
                  statusLabel={
                    names.has(node.characterId) && near
                      ? characterStatusLabel(node, labels)
                      : undefined
                  }
                />
              </g>
            ))}
          </g>
        </g>
      </svg>
    );
  },
);

function SvgCharacter({
  node,
  selected,
}: {
  readonly node: SceneCharacter;
  readonly selected: boolean;
}) {
  return (
    <>
      <ellipse
        cx="0"
        cy="1"
        rx="21"
        ry="8"
        fill={characterShadow.color}
        opacity={characterShadow.alpha}
      />
      {selected ? (
        <g fill="none">
          <ellipse
            cx="0"
            cy="1"
            rx="30"
            ry="13"
            stroke={selectionRing.outer}
            strokeWidth={selectionRing.outerWidth}
          />
          <ellipse
            cx="0"
            cy="1"
            rx="30"
            ry="13"
            stroke={selectionRing.inner}
            strokeWidth={selectionRing.innerWidth}
          />
        </g>
      ) : null}
      <g opacity={node.status === 'offline' ? 0.56 : 1}>
        {node.kind === 'human' ? (
          <g>
            <polygon points="-28,-32 -14,-57 14,-57 28,-32 14,-7 -14,-7" fill={humanMarker.fill} />
            <circle cx="0" cy="-32" r="10" fill={humanMarker.mark} />
          </g>
        ) : (
          <StudioSprite id={node.characterId} />
        )}
        <circle
          cx="30"
          cy="-70"
          r="4.5"
          fill={statusStickerFill[node.status]}
          stroke={sceneInk}
          strokeWidth="2"
        />
      </g>
      {node.status === 'waiting_input' || node.status === 'blocked' ? (
        <g>
          <rect
            x="25"
            y="-106"
            width="23"
            height="23"
            rx="8"
            fill={statusBadgeFill[node.status]}
            stroke={sceneInk}
            strokeWidth="2"
          />
          <text
            x="37"
            y="-90"
            textAnchor="middle"
            style={{ fill: sceneInk, fontSize: 19, fontWeight: 700 }}
          >
            {node.status === 'blocked' ? '!' : '?'}
          </text>
        </g>
      ) : null}
      <rect x="-38" y="-92" width="76" height="132" fill="transparent" />
    </>
  );
}

function SvgCharacterLabels({
  node,
  visible,
  statusLabel,
}: {
  readonly node: SceneCharacter;
  readonly visible: boolean;
  readonly statusLabel: string | undefined;
}) {
  const name = characterLabel(node.displayName);
  const nameWidth = estimatedTextWidth(name, nameplate.fontSize) + nameplate.paddingX * 2;
  const statusTop = nameplate.top + nameplate.height + statusSticker.gap;
  const statusWidth =
    statusLabel === undefined
      ? 0
      : estimatedTextWidth(statusLabel, statusSticker.fontSize) + statusSticker.paddingX * 2;
  return (
    <>
      <rect
        className="room-character-plate"
        data-visible={visible}
        x={-nameWidth / 2}
        y={nameplate.top}
        width={nameWidth}
        height={nameplate.height}
        rx={nameplate.height / 2}
        fill={nameplate.fill}
        stroke={nameplate.stroke}
        strokeWidth={nameplate.strokeWidth}
      />
      <text
        className="room-character-name"
        textAnchor="middle"
        dominantBaseline="central"
        y={nameplate.top + nameplate.height / 2}
        data-visible={visible}
      >
        {name}
      </text>
      {statusLabel === undefined || node.kind === 'human' ? null : (
        <g>
          <rect
            x={-statusWidth / 2}
            y={statusTop}
            width={statusWidth}
            height={statusSticker.height}
            rx={statusSticker.height / 2}
            fill={statusStickerFill[node.status]}
            stroke={statusSticker.stroke}
            strokeWidth={statusSticker.strokeWidth}
          />
          <text
            x="0"
            y={statusTop + statusSticker.height / 2}
            textAnchor="middle"
            dominantBaseline="central"
            className="room-character-status"
          >
            {statusLabel}
          </text>
        </g>
      )}
    </>
  );
}
import { characterStatusLabel } from '../lobby-scene';
