// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { projectLobbyScene } from '@/features/lobby/domain/scene-projection';
import { LobbySceneSurface } from '@/features/lobby/ui/lobby-scene-surface';

vi.mock('@/features/lobby/scene/pixi/pixi-lobby-scene', () => ({
  mountPixiLobbyScene: vi.fn().mockRejectedValue(new Error('WebGL unavailable')),
}));

class ResizeObserverStub implements ResizeObserver {
  readonly #callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.#callback = callback;
  }

  disconnect(): void {}

  observe(target: Element): void {
    this.#callback(
      [
        {
          borderBoxSize: [],
          contentBoxSize: [],
          contentRect: target.getBoundingClientRect(),
          devicePixelContentBoxSize: [],
          target,
        },
      ],
      this,
    );
  }

  unobserve(): void {}
}

beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue({
    bottom: 720,
    height: 720,
    left: 0,
    right: 1_280,
    toJSON: () => ({}),
    top: 0,
    width: 1_280,
    x: 0,
    y: 0,
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('LobbySceneSurface', () => {
  it('Pixi 初始化失败时自动降级为可交互 SVG 空间视图', async () => {
    const onSelectAgent = vi.fn();
    const view = render(
      <LobbySceneSurface
        labels={{
          canvas: 'Agent spatial view',
          zones: { active: 'Active', attention: 'Attention', available: 'Available' },
        }}
        languageKey="en"
        onSelectAgent={onSelectAgent}
        onZoomChange={vi.fn()}
        projection={projectLobbyScene(
          {
            agents: [
              {
                agentId: 'agent-local',
                displayName: 'Local Agent',
                instanceIds: ['instance-local'],
                matrixUserId: '@local:agent-room.test',
                status: 'working',
                statusExpiresAtUnixMs: 1_800_000_000_000,
                trust: 'verified',
                visibility: 'coarse',
              },
            ],
            name: 'Public lobby',
            observedAtUnixMs: 1_700_000_000_000,
            roomId: '!public:agent-room.test',
          },
          null,
        )}
      />,
    );

    const svg = await waitFor(() => {
      const element = view.container.querySelector<SVGSVGElement>('[data-renderer="svg"]');
      expect(element).not.toBeNull();
      return element as SVGSVGElement;
    });
    expect(svg).toHaveTextContent('Local Agent');

    const agent = view.container.querySelector<SVGGElement>('.lobby-scene__svg-agent');
    expect(agent).not.toBeNull();
    fireEvent.click(agent as SVGGElement);
    expect(onSelectAgent).toHaveBeenCalledWith('agent-local');
  });
});
