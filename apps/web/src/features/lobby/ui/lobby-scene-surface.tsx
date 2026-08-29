import {
  forwardRef,
  useEffect,
  useId,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';

import type { LobbySceneLabels } from '@/features/lobby/scene/lobby-scene';
import type { LobbySceneHandle } from '@/features/lobby/scene/lobby-scene';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import { nextAgentInDirection } from '@/features/lobby/domain/spatial-navigation';
import {
  SceneSelectionAnnouncement,
  SceneSemanticRoster,
} from '@/features/lobby/ui/scene-semantic-roster';

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
  readonly onSceneInitialized?: (durationMs: number) => void;
  readonly onZoomChange: (zoom: number) => void;
  readonly projection: LobbySceneProjection;
};

export const LobbySceneSurface = forwardRef<LobbySceneSurfaceHandle, LobbySceneSurfaceProps>(
  function LobbySceneSurface(
    { labels, languageKey, onFailure, onSceneInitialized, onSelectAgent, onZoomChange, projection },
    forwardedRef,
  ) {
    const hostRef = useRef<HTMLDivElement>(null);
    const canvasHostRef = useRef<HTMLDivElement>(null);
    const handleRef = useRef<LobbySceneHandle | null>(null);
    const [activeAgentId, setActiveAgentId] = useState<string | null>(
      projection.selectedAgentId ?? projection.nodes[0]?.agentId ?? null,
    );
    const semanticId = useId().replaceAll(':', '');
    const instructionsId = `lobby-scene-instructions-${semanticId}`;
    const optionId = (agentId: string): string => `lobby-scene-${semanticId}-${agentId}`;
    const activeAgentExists = projection.nodes.some((node) => node.agentId === activeAgentId);
    const normalizedActiveAgentId = activeAgentExists
      ? activeAgentId
      : (projection.selectedAgentId ?? projection.nodes[0]?.agentId ?? null);
    const sceneProjection = useMemo(
      () => ({ ...projection, selectedAgentId: normalizedActiveAgentId }),
      [normalizedActiveAgentId, projection],
    );
    const projectionRef = useRef(projection);
    const selectRef = useRef(onSelectAgent);
    const zoomRef = useRef(onZoomChange);
    const failureRef = useRef(onFailure);
    const initializedRef = useRef(onSceneInitialized);
    projectionRef.current = sceneProjection;
    selectRef.current = onSelectAgent;
    zoomRef.current = onZoomChange;
    failureRef.current = onFailure;
    initializedRef.current = onSceneInitialized;

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
      const host = canvasHostRef.current;
      if (host === null) {
        return undefined;
      }
      const startedAt = performance.now();
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
            initializedRef.current?.(performance.now() - startedAt);
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
      handleRef.current?.update(sceneProjection);
    }, [sceneProjection]);

    useEffect(() => {
      if (projection.selectedAgentId !== null) {
        setActiveAgentId(projection.selectedAgentId);
      } else if (!activeAgentExists) {
        setActiveAgentId(projection.nodes[0]?.agentId ?? null);
      }
    }, [activeAgentExists, projection.nodes, projection.selectedAgentId]);

    const moveActiveAgent = (key: string): boolean => {
      const direction = directionForKey(key);
      if (direction === null) {
        return false;
      }
      setActiveAgentId(nextAgentInDirection(projection.nodes, normalizedActiveAgentId, direction));
      return true;
    };

    return (
      <div className="lobby-scene-frame">
        <div
          aria-activedescendant={
            normalizedActiveAgentId === null ? undefined : optionId(normalizedActiveAgentId)
          }
          aria-describedby={instructionsId}
          aria-label={labels.canvas}
          className="lobby-scene"
          onKeyDown={(event) => {
            if (moveActiveAgent(event.key)) {
              event.preventDefault();
            } else if (event.key === 'Escape') {
              event.preventDefault();
              onSelectAgent(null);
            } else if (
              (event.key === 'Enter' || event.key === ' ') &&
              normalizedActiveAgentId !== null
            ) {
              event.preventDefault();
              onSelectAgent(normalizedActiveAgentId);
            }
          }}
          ref={hostRef}
          role="listbox"
          tabIndex={0}
        >
          <div aria-hidden="true" className="lobby-scene__visual" ref={canvasHostRef} />
          <SceneSemanticRoster
            activeAgentId={normalizedActiveAgentId}
            nodes={projection.nodes}
            optionId={optionId}
          />
        </div>
        <SceneSelectionAnnouncement
          activeAgentId={normalizedActiveAgentId}
          instructionsId={instructionsId}
          nodes={projection.nodes}
        />
      </div>
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
