import { SceneSpeechLayer, type SceneSpeechLayerHandle } from './scene-speech-layer';
import type { RoomSpeech } from '../domain/room-speech';
import type { SceneFrame } from '../scene/scene-character';
import {
  forwardRef,
  useCallback,
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
  SvgLobbyScene,
  type SvgLobbySceneHandle,
} from '@/features/lobby/scene/svg/svg-lobby-scene';
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
  readonly speech?: readonly RoomSpeech[];
  readonly onOpenSpeech?: (id: string) => void;
  readonly onSelectHuman?: (id: string) => void;
  readonly labels: LobbySceneLabels;
  readonly languageKey: string;
  readonly onSelectAgent: (agentId: string | null) => void;
  readonly onSceneInitialized?: (durationMs: number) => void;
  readonly onZoomChange: (zoom: number) => void;
  readonly projection: LobbySceneProjection;
};

export const LobbySceneSurface = forwardRef<LobbySceneSurfaceHandle, LobbySceneSurfaceProps>(
  function LobbySceneSurface(
    {
      labels,
      languageKey,
      onSceneInitialized,
      onSelectAgent,
      onZoomChange,
      projection,
      speech = [],
      onOpenSpeech,
      onSelectHuman,
    },
    forwardedRef,
  ) {
    const bubbleLayer = useRef<SceneSpeechLayerHandle>(null);
    const humanSelection = useRef(onSelectHuman);
    humanSelection.current = onSelectHuman;
    const onFrame = useCallback((frame: SceneFrame): void => {
      bubbleLayer.current?.position(frame);
    }, []);
    const [keyboardFocus, setKeyboardFocus] = useState(false);
    const externalSelection = useRef(projection.selectedAgentId);
    externalSelection.current = projection.selectedAgentId;
    const hostRef = useRef<HTMLDivElement>(null);
    const canvasHostRef = useRef<HTMLDivElement>(null);
    const handleRef = useRef<LobbySceneHandle | null>(null);
    const svgHandleRef = useRef<SvgLobbySceneHandle | null>(null);
    const [renderer, setRenderer] = useState<'pixi' | 'svg'>('pixi');
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
      () => ({
        ...projection,
        selectedAgentId:
          projection.selectedAgentId ?? (keyboardFocus ? normalizedActiveAgentId : null),
      }),
      [keyboardFocus, normalizedActiveAgentId, projection],
    );
    const projectionRef = useRef(projection);
    const selectRef = useRef(onSelectAgent);
    const zoomRef = useRef(onZoomChange);
    const initializedRef = useRef(onSceneInitialized);
    projectionRef.current = sceneProjection;
    selectRef.current = onSelectAgent;
    zoomRef.current = onZoomChange;
    initializedRef.current = onSceneInitialized;

    useImperativeHandle(forwardedRef, () => ({
      focus: () => {
        hostRef.current?.focus();
      },
      resetViewport: () => {
        if (renderer === 'pixi') handleRef.current?.resetViewport();
        else svgHandleRef.current?.resetViewport();
      },
      zoomBy: (factor) => {
        if (renderer === 'pixi') handleRef.current?.zoomBy(factor);
        else svgHandleRef.current?.zoomBy(factor);
      },
    }));

    useEffect(() => {
      if (renderer !== 'pixi') return undefined;
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
            onFrame,
            onSelectHuman: (id) => {
              humanSelection.current?.(id);
            },
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
            handle.update(projectionRef.current);
            if (externalSelection.current !== null) handle.focusAgent?.(externalSelection.current);
            initializedRef.current?.(performance.now() - startedAt);
          }
        })
        .catch(() => {
          if (!disposed) {
            setRenderer('svg');
          }
        });
      return () => {
        disposed = true;
        handleRef.current?.destroy();
        handleRef.current = null;
      };
    }, [labels, languageKey, renderer, onFrame]);

    useEffect(() => {
      handleRef.current?.update(sceneProjection);
    }, [sceneProjection]);

    useEffect(() => {
      if (projection.selectedAgentId !== null) {
        if (renderer === 'pixi') handleRef.current?.focusAgent?.(projection.selectedAgentId);
        else svgHandleRef.current?.focusAgent(projection.selectedAgentId);
      }
    }, [projection.selectedAgentId, renderer]);

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
          onFocus={() => {
            setKeyboardFocus(true);
          }}
          onBlur={() => {
            setKeyboardFocus(false);
          }}
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
          <div aria-hidden="true" className="lobby-scene__visual">
            {renderer === 'pixi' ? (
              <div className="lobby-scene__pixi" data-renderer="pixi" ref={canvasHostRef} />
            ) : (
              <SvgLobbyScene
                labels={labels}
                onFrame={onFrame}
                onSelectHuman={(id) => {
                  humanSelection.current?.(id);
                }}
                onSelectAgent={onSelectAgent}
                onZoomChange={onZoomChange}
                projection={sceneProjection}
                ref={svgHandleRef}
              />
            )}
          </div>
          <SceneSemanticRoster
            activeAgentId={normalizedActiveAgentId}
            nodes={projection.nodes}
            optionId={optionId}
          />
        </div>
        <SceneSpeechLayer
          speech={speech}
          onOpen={(id) => {
            onOpenSpeech?.(id);
          }}
          ref={bubbleLayer}
        />
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
