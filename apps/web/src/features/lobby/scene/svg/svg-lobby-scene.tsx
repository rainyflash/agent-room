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

import { monogram } from '@/features/lobby/scene/pixi/agent-node-view';
import type { LobbySceneLabels } from '@/features/lobby/scene/lobby-scene';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import {
  ViewportController,
  type CameraSnapshot,
} from '@/features/lobby/scene/viewport-controller';

export type SvgLobbySceneHandle = {
  resetViewport(): void;
  zoomBy(factor: number): void;
};

export type SvgLobbySceneProps = {
  readonly labels: LobbySceneLabels;
  readonly onSelectAgent: (agentId: string | null) => void;
  readonly onZoomChange: (zoom: number) => void;
  readonly projection: LobbySceneProjection;
};

export const SvgLobbyScene = forwardRef<SvgLobbySceneHandle, SvgLobbySceneProps>(
  function SvgLobbyScene({ labels, onSelectAgent, onZoomChange, projection }, forwardedRef) {
    const hostRef = useRef<SVGSVGElement>(null);
    const controllerRef = useRef<ViewportController | null>(null);
    const dragRef = useRef<{ readonly x: number; readonly y: number; moved: boolean } | null>(null);
    const [camera, setCamera] = useState<CameraSnapshot>({ scale: 1, x: 0, y: 0 });
    const [viewport, setViewport] = useState({ height: 1, width: 1 });

    controllerRef.current ??= new ViewportController(projection.world);

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
        resetViewport: () => {
          const controller = controllerRef.current;
          if (controller !== null) commitCamera(controller.reset());
        },
        zoomBy: (factor) => {
          const controller = controllerRef.current;
          if (controller !== null) commitCamera(controller.zoomBy(factor));
        },
      }),
      [commitCamera],
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
      if (typeof ResizeObserver === 'undefined') {
        window.addEventListener('resize', resize);
        return () => {
          window.removeEventListener('resize', resize);
        };
      }
      const observer = new ResizeObserver(resize);
      observer.observe(host);
      return () => {
        observer.disconnect();
      };
    }, [commitCamera]);

    const pointerPosition = (
      event: ReactPointerEvent<SVGSVGElement> | ReactWheelEvent<SVGSVGElement>,
    ): { readonly x: number; readonly y: number } => {
      const bounds = event.currentTarget.getBoundingClientRect();
      return { x: event.clientX - bounds.left, y: event.clientY - bounds.top };
    };

    return (
      <svg
        aria-hidden="true"
        className="lobby-scene__svg"
        data-renderer="svg"
        onClick={() => {
          if (dragRef.current?.moved !== true) onSelectAgent(null);
          dragRef.current = null;
        }}
        onPointerDown={(event) => {
          const point = pointerPosition(event);
          dragRef.current = { ...point, moved: false };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          const drag = dragRef.current;
          const controller = controllerRef.current;
          if (drag === null || controller === null) return;
          const point = pointerPosition(event);
          const deltaX = point.x - drag.x;
          const deltaY = point.y - drag.y;
          if (Math.abs(deltaX) + Math.abs(deltaY) > 2) drag.moved = true;
          dragRef.current = { x: point.x, y: point.y, moved: drag.moved };
          commitCamera(controller.panBy(deltaX, deltaY));
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
        }}
        onPointerCancel={() => {
          dragRef.current = null;
        }}
        onWheel={(event) => {
          event.preventDefault();
          const controller = controllerRef.current;
          if (controller === null) return;
          const point = pointerPosition(event);
          commitCamera(controller.zoomBy(Math.exp(-event.deltaY * 0.0012), point.x, point.y));
        }}
        ref={hostRef}
        viewBox={`0 0 ${String(viewport.width)} ${String(viewport.height)}`}
      >
        <g
          transform={`translate(${String(camera.x)} ${String(camera.y)}) scale(${String(camera.scale)})`}
        >
          {projection.zones.map((zone) => (
            <g className={`lobby-scene__svg-zone lobby-scene__svg-zone--${zone.id}`} key={zone.id}>
              <rect
                height={zone.height}
                rx="34"
                vectorEffect="non-scaling-stroke"
                width={zone.width}
                x={zone.x}
                y={zone.y}
              />
              <text x={zone.x + 24} y={zone.y + 37}>
                {labels.zones[zone.id].toLocaleUpperCase()}
              </text>
            </g>
          ))}
          {projection.nodes.map((node) => (
            <g
              className={`lobby-scene__svg-agent lobby-scene__svg-agent--${node.status}`}
              data-selected={node.agentId === projection.selectedAgentId ? 'true' : 'false'}
              key={node.agentId}
              onClick={(event) => {
                event.stopPropagation();
                if (dragRef.current?.moved !== true) onSelectAgent(node.agentId);
                dragRef.current = null;
              }}
              transform={`translate(${String(node.x)} ${String(node.y)})`}
            >
              <circle className="lobby-scene__svg-agent-ring" r={node.radius + 5} />
              <circle className="lobby-scene__svg-agent-body" r={node.radius} />
              <text className="lobby-scene__svg-agent-mark" textAnchor="middle" y="7">
                {monogram(node.displayName)}
              </text>
              <text
                className="lobby-scene__svg-agent-name"
                textAnchor="middle"
                y={node.radius + 30}
              >
                {node.displayName}
              </text>
            </g>
          ))}
        </g>
      </svg>
    );
  },
);
