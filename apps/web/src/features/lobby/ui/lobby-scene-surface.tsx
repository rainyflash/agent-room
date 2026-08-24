import { forwardRef, useEffect, useImperativeHandle, useRef } from 'react';

import type { LobbySceneLabels } from '@/features/lobby/scene/lobby-scene';
import type { LobbySceneHandle } from '@/features/lobby/scene/lobby-scene';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import { nextAgentInDirection } from '@/features/lobby/domain/spatial-navigation';

export type LobbySceneSurfaceHandle = {
  focus(): void;
  resetViewport(): void;
  zoomBy(factor: number): void;
};

export type LobbySceneSurfaceProps = {
  readonly labels: LobbySceneLabels;
  readonly languageKey: string;
  readonly onFailure: () => void;
  readonly onSelectAgent: (agentId: string | null) => void;
  readonly onZoomChange: (zoom: number) => void;
  readonly projection: LobbySceneProjection;
};

export const LobbySceneSurface = forwardRef<LobbySceneSurfaceHandle, LobbySceneSurfaceProps>(
  function LobbySceneSurface(
    { labels, languageKey, onFailure, onSelectAgent, onZoomChange, projection },
    forwardedRef,
  ) {
    const hostRef = useRef<HTMLDivElement>(null);
    const handleRef = useRef<LobbySceneHandle | null>(null);
    const projectionRef = useRef(projection);
    const selectRef = useRef(onSelectAgent);
    const zoomRef = useRef(onZoomChange);
    const failureRef = useRef(onFailure);
    projectionRef.current = projection;
    selectRef.current = onSelectAgent;
    zoomRef.current = onZoomChange;
    failureRef.current = onFailure;

    useImperativeHandle(forwardedRef, () => ({
      focus: () => {
        hostRef.current?.focus();
      },
      resetViewport: () => {
        handleRef.current?.resetViewport();
      },
      zoomBy: (factor) => {
        handleRef.current?.zoomBy(factor);
      },
    }));

    useEffect(() => {
      const host = hostRef.current;
      if (host === null) {
        return undefined;
      }
      let disposed = false;
      void import('@/features/lobby/scene/pixi/pixi-lobby-scene')
        .then(async ({ mountPixiLobbyScene }) => {
          const handle = await mountPixiLobbyScene({
            host,
            labels,
            onSelectAgent: (agentId) => {
              selectRef.current(agentId);
            },
            onZoomChange: (zoom) => {
              zoomRef.current(zoom);
            },
            projection: projectionRef.current,
          });
          if (disposed) {
            handle.destroy();
          } else {
            handleRef.current = handle;
          }
        })
        .catch(() => {
          if (!disposed) {
            failureRef.current();
          }
        });
      return () => {
        disposed = true;
        handleRef.current?.destroy();
        handleRef.current = null;
      };
    }, [labels, languageKey]);

    useEffect(() => {
      handleRef.current?.update(projection);
    }, [projection]);

    return (
      <div
        aria-label={labels.canvas}
        className="lobby-scene"
        onKeyDown={(event) => {
          const direction = directionForKey(event.key);
          if (direction !== null) {
            event.preventDefault();
            const firstAgentId = projection.nodes[0]?.agentId ?? null;
            const nextAgentId =
              projection.selectedAgentId === null
                ? firstAgentId
                : nextAgentInDirection(projection.nodes, projection.selectedAgentId, direction);
            onSelectAgent(nextAgentId);
          } else if (event.key === 'Escape') {
            event.preventDefault();
            onSelectAgent(null);
          } else if (event.key === 'Enter' && projection.selectedAgentId !== null) {
            event.preventDefault();
            onSelectAgent(projection.selectedAgentId);
          }
        }}
        ref={hostRef}
        role="application"
        tabIndex={0}
      />
    );
  },
);

function directionForKey(key: string) {
  const directionByKey = {
    ArrowDown: 'down',
    ArrowLeft: 'left',
    ArrowRight: 'right',
    ArrowUp: 'up',
  } as const;
  return key in directionByKey ? directionByKey[key as keyof typeof directionByKey] : null;
}
