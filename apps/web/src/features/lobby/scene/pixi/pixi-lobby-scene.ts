import type { Application, Container, FederatedPointerEvent, FederatedWheelEvent } from 'pixi.js';

import { createAgentNodeView } from './agent-node-view';
import { createZoneLayer } from './zone-layer';
import { sceneDetailForZoom, visibleLobbyNodes } from '@/features/lobby/domain/scene-projection';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import type { LobbySceneHandle, LobbySceneMountOptions } from '@/features/lobby/scene/lobby-scene';
import { ViewportController } from '@/features/lobby/scene/viewport-controller';

type PixiModule = typeof import('pixi.js');

export async function mountPixiLobbyScene(
  options: LobbySceneMountOptions,
): Promise<LobbySceneHandle> {
  if (!supportsWebGl()) {
    throw new Error('当前浏览器没有可用的 WebGL 图形上下文。');
  }
  const pixi = await import('pixi.js');
  const scene = new PixiLobbyScene(pixi, options);
  await scene.initialize();
  return scene;
}

function supportsWebGl(): boolean {
  const probe = document.createElement('canvas');
  return probe.getContext('webgl2') !== null || probe.getContext('webgl') !== null;
}

class PixiLobbyScene implements LobbySceneHandle {
  readonly #callbacks: Pick<LobbySceneMountOptions, 'onSelectAgent' | 'onZoomChange'>;
  readonly #camera: ViewportController;
  readonly #host: HTMLElement;
  readonly #labels: LobbySceneMountOptions['labels'];
  readonly #pixi: PixiModule;
  #app: Application | null = null;
  #destroyed = false;
  #dragOrigin: { readonly pointerX: number; readonly pointerY: number } | null = null;
  #frame: number | null = null;
  #nodesLayer: Container | null = null;
  #projection: LobbySceneProjection;
  #resizeObserver: ResizeObserver | null = null;
  #worldLayer: Container | null = null;

  constructor(pixi: PixiModule, options: LobbySceneMountOptions) {
    this.#pixi = pixi;
    this.#host = options.host;
    this.#projection = options.projection;
    this.#labels = options.labels;
    this.#callbacks = options;
    this.#camera = new ViewportController(options.projection.world);
  }

  async initialize(): Promise<void> {
    const bounds = this.#host.getBoundingClientRect();
    const app = new this.#pixi.Application();
    await app.init({
      antialias: true,
      autoDensity: true,
      autoStart: false,
      backgroundAlpha: 0,
      height: Math.max(1, bounds.height),
      preference: 'webgl',
      resolution: Math.min(window.devicePixelRatio || 1, 2),
      sharedTicker: false,
      width: Math.max(1, bounds.width),
    });
    if (this.#destroyed) {
      app.destroy({ removeView: true }, { children: true, context: true });
      return;
    }

    this.#app = app;
    app.canvas.setAttribute('aria-hidden', 'true');
    app.canvas.className = 'lobby-scene__canvas';
    this.#host.replaceChildren(app.canvas);
    app.stage.eventMode = 'static';
    app.stage.hitArea = app.screen;
    app.stage.on('pointerdown', this.#handlePointerDown);
    app.stage.on('pointermove', this.#handlePointerMove);
    app.stage.on('pointerup', this.#handlePointerUp);
    app.stage.on('pointerupoutside', this.#handlePointerUp);
    app.stage.on('pointertap', this.#handleBackgroundSelect);
    app.stage.on('wheel', this.#handlePixiWheel);

    const worldLayer = new this.#pixi.Container();
    const nodesLayer = new this.#pixi.Container();
    worldLayer.addChild(createZoneLayer(this.#pixi, this.#projection, this.#labels.zones));
    worldLayer.addChild(nodesLayer);
    app.stage.addChild(worldLayer);
    this.#worldLayer = worldLayer;
    this.#nodesLayer = nodesLayer;

    this.#camera.resize(app.screen.width, app.screen.height);
    this.#callbacks.onZoomChange(this.#camera.snapshot().scale);
    this.#resizeObserver = new ResizeObserver(() => {
      const nextBounds = this.#host.getBoundingClientRect();
      app.renderer.resize(Math.max(1, nextBounds.width), Math.max(1, nextBounds.height));
      app.stage.hitArea = app.screen;
      this.#camera.resize(app.screen.width, app.screen.height);
      this.#scheduleRender();
    });
    this.#resizeObserver.observe(this.#host);
    this.#renderNow();
  }

  destroy(): void {
    this.#destroyed = true;
    if (this.#frame !== null) {
      window.cancelAnimationFrame(this.#frame);
      this.#frame = null;
    }
    this.#resizeObserver?.disconnect();
    this.#resizeObserver = null;
    const app = this.#app;
    this.#app = null;
    this.#worldLayer = null;
    this.#nodesLayer = null;
    app?.destroy({ removeView: true }, { children: true, context: true });
  }

  resetViewport(): void {
    this.#callbacks.onZoomChange(this.#camera.reset().scale);
    this.#scheduleRender();
  }

  update(projection: LobbySceneProjection): void {
    this.#projection = projection;
    this.#scheduleRender();
  }

  zoomBy(factor: number): void {
    this.#callbacks.onZoomChange(this.#camera.zoomBy(factor).scale);
    this.#scheduleRender();
  }

  readonly #handleBackgroundSelect = (): void => {
    this.#callbacks.onSelectAgent(null);
  };

  readonly #handlePixiWheel = (event: FederatedWheelEvent): void => {
    event.preventDefault();
    const factor = Math.exp(-event.deltaY * 0.0012);
    const camera = this.#camera.zoomBy(factor, event.global.x, event.global.y);
    this.#callbacks.onZoomChange(camera.scale);
    this.#scheduleRender();
  };

  readonly #handlePointerDown = (event: FederatedPointerEvent): void => {
    this.#dragOrigin = { pointerX: event.global.x, pointerY: event.global.y };
  };

  readonly #handlePointerMove = (event: FederatedPointerEvent): void => {
    const origin = this.#dragOrigin;
    if (origin === null) {
      return;
    }
    this.#camera.panBy(event.global.x - origin.pointerX, event.global.y - origin.pointerY);
    this.#dragOrigin = { pointerX: event.global.x, pointerY: event.global.y };
    this.#scheduleRender();
  };

  readonly #handlePointerUp = (): void => {
    this.#dragOrigin = null;
  };

  #renderNow(): void {
    const app = this.#app;
    const nodesLayer = this.#nodesLayer;
    const worldLayer = this.#worldLayer;
    if (app === null || nodesLayer === null || worldLayer === null) {
      return;
    }
    const camera = this.#camera.snapshot();
    worldLayer.position.set(camera.x, camera.y);
    worldLayer.scale.set(camera.scale);
    for (const child of nodesLayer.removeChildren()) {
      child.destroy({ children: true });
    }
    const detail = sceneDetailForZoom(camera.scale);
    for (const node of visibleLobbyNodes(this.#projection, this.#camera.viewport())) {
      nodesLayer.addChild(
        createAgentNodeView(this.#pixi, {
          detail,
          node,
          onSelect: this.#callbacks.onSelectAgent,
          selected: node.agentId === this.#projection.selectedAgentId,
        }),
      );
    }
    app.render();
  }

  #scheduleRender(): void {
    if (this.#frame !== null || this.#destroyed) {
      return;
    }
    this.#frame = window.requestAnimationFrame(() => {
      this.#frame = null;
      this.#renderNow();
    });
  }
}
