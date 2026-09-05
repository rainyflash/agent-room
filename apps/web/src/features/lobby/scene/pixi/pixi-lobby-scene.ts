import { sceneCharacters, type SceneFrame } from '../scene-character';
import type { Application, Container, FederatedPointerEvent, FederatedWheelEvent } from 'pixi.js';
import { createAgentNodeView, type AgentCharacterView } from './agent-node-view';
import { createRoomProps, createZoneLayer } from './zone-layer';
import {
  sceneDetailForZoom,
  visibleLobbyNodes,
  type LobbySceneDetail,
  type LobbySceneProjection,
} from '@/features/lobby/domain/scene-projection';
import { characterPose } from '../character-motion';
import type { LobbySceneHandle, LobbySceneMountOptions } from '@/features/lobby/scene/lobby-scene';
import { ViewportController } from '@/features/lobby/scene/viewport-controller';

type PixiModule = typeof import('pixi.js');
type TextureAwareRenderer = {
  readonly texture?: { readonly managedTextures?: readonly unknown[] };
};
type PointerPosition = { readonly x: number; readonly y: number };

export async function mountPixiLobbyScene(
  options: LobbySceneMountOptions,
): Promise<LobbySceneHandle> {
  const probe = document.createElement('canvas');
  if (probe.getContext('webgl2') === null && probe.getContext('webgl') === null)
    throw new Error('当前浏览器没有可用的 WebGL 图形上下文。');
  const scene = new PixiLobbyScene(await import('pixi.js'), options);
  await scene.initialize();
  return scene;
}

class PixiLobbyScene implements LobbySceneHandle {
  readonly #callbacks: Pick<
    LobbySceneMountOptions,
    'onSelectAgent' | 'onZoomChange' | 'onFrame' | 'onSelectHuman'
  >;
  readonly #camera: ViewportController;
  readonly #host: HTMLElement;
  readonly #labels: LobbySceneMountOptions['labels'];
  readonly #pixi: PixiModule;
  readonly #views = new Map<
    string,
    { readonly view: AgentCharacterView; readonly signature: string }
  >();
  readonly #pointers = new Map<number, PointerPosition>();
  readonly #motion = window.matchMedia('(prefers-reduced-motion: reduce)');
  #app: Application | null = null;
  #destroyed = false;
  #gestureMoved = false;
  #frame: number | null = null;
  #animationFrame: number | null = null;
  #elapsedSeconds = 0;
  #lastAnimationAt = 0;
  #objectsLayer: Container | null = null;
  #projection: LobbySceneProjection;
  #resizeObserver: ResizeObserver | null = null;
  #worldLayer: Container | null = null;

  constructor(pixi: PixiModule, options: LobbySceneMountOptions) {
    this.#pixi = pixi;
    this.#host = options.host;
    this.#projection = options.projection;
    this.#labels = options.labels;
    this.#callbacks = options;
    this.#camera = new ViewportController(options.projection.world, {
      padding: 22,
      minimumScale: 0.22,
    });
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
    app.stage.on('globalpointermove', this.#handlePointerMove);
    app.stage.on('pointerup', this.#handlePointerUp);
    app.stage.on('pointerupoutside', this.#handlePointerUp);
    app.stage.on('pointercancel', this.#handlePointerUp);
    app.stage.on('pointertap', this.#handleBackgroundSelect);
    app.stage.on('wheel', this.#handlePixiWheel);
    const world = new this.#pixi.Container();
    const objects = new this.#pixi.Container();
    objects.sortableChildren = true;
    objects.addChild(...createRoomProps(this.#pixi));
    world.addChild(createZoneLayer(this.#pixi, this.#projection, this.#labels.zones), objects);
    app.stage.addChild(world);
    this.#worldLayer = world;
    this.#objectsLayer = objects;
    this.#camera.resize(app.screen.width, app.screen.height);
    this.#callbacks.onZoomChange(this.#camera.snapshot().scale);
    this.#resizeObserver = new ResizeObserver(() => {
      const next = this.#host.getBoundingClientRect();
      app.renderer.resize(Math.max(1, next.width), Math.max(1, next.height));
      app.stage.hitArea = app.screen;
      this.#camera.resize(app.screen.width, app.screen.height);
      this.#scheduleRender();
    });
    this.#resizeObserver.observe(this.#host);
    document.addEventListener('visibilitychange', this.#syncAnimation);
    this.#motion.addEventListener('change', this.#syncAnimation);
    this.#renderNow();
    this.#syncAnimation();
  }

  destroy(): void {
    this.#destroyed = true;
    if (this.#frame !== null) window.cancelAnimationFrame(this.#frame);
    if (this.#animationFrame !== null) window.cancelAnimationFrame(this.#animationFrame);
    this.#resizeObserver?.disconnect();
    document.removeEventListener('visibilitychange', this.#syncAnimation);
    this.#motion.removeEventListener('change', this.#syncAnimation);
    for (const { view } of this.#views.values()) view.destroy();
    this.#views.clear();
    this.#pointers.clear();
    const app = this.#app;
    this.#app = null;
    this.#worldLayer = null;
    this.#objectsLayer = null;
    for (const key of [
      'agentRoomRenderedNodes',
      'agentRoomRenderMilliseconds',
      'agentRoomRenderSequence',
      'agentRoomUpdateMilliseconds',
      'agentRoomTextureCount',
      'agentRoomAnimationFrame',
      'agentRoomMotion',
    ])
      Reflect.deleteProperty(this.#host.dataset, key);
    app?.destroy({ removeView: true }, { children: true, context: true });
  }

  focusAgent(agentId: string): void {
    const node = this.#projection.nodes.find((candidate) => candidate.agentId === agentId);
    if (node === undefined) return;
    this.#callbacks.onZoomChange(this.#camera.focusOn(node.x, node.y - 35).scale);
    this.#scheduleRender();
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

  readonly #syncAnimation = (): void => {
    if (this.#animationFrame !== null) window.cancelAnimationFrame(this.#animationFrame);
    this.#animationFrame = null;
    const active = !this.#destroyed && !document.hidden && !this.#motion.matches;
    this.#host.dataset.agentRoomMotion = active ? 'active' : 'paused';
    this.#lastAnimationAt = performance.now();
    if (active) this.#animationFrame = window.requestAnimationFrame(this.#animate);
    else if (!this.#destroyed) this.#scheduleRender();
  };

  readonly #animate = (now: number): void => {
    this.#animationFrame = null;
    if (this.#destroyed || document.hidden || this.#motion.matches) return;
    const elapsed = now - this.#lastAnimationAt;
    if (elapsed >= 1000 / 30) {
      this.#elapsedSeconds += Math.min(elapsed, 100) / 1000;
      this.#lastAnimationAt = now;
      this.#renderNow(true);
      this.#host.dataset.agentRoomAnimationFrame = String(
        Number(this.#host.dataset.agentRoomAnimationFrame ?? '0') + 1,
      );
    }
    this.#animationFrame = window.requestAnimationFrame(this.#animate);
  };

  readonly #selectAgent = (id: string): void => {
    if (!this.#gestureMoved) this.#callbacks.onSelectAgent(id);
  };
  readonly #handleBackgroundSelect = (): void => {
    if (!this.#gestureMoved) this.#callbacks.onSelectAgent(null);
  };
  readonly #handlePixiWheel = (event: FederatedWheelEvent): void => {
    event.preventDefault();
    this.#callbacks.onZoomChange(
      this.#camera.zoomBy(Math.exp(-event.deltaY * 0.0012), event.global.x, event.global.y).scale,
    );
    this.#scheduleRender();
  };
  readonly #handlePointerDown = (event: FederatedPointerEvent): void => {
    if (this.#pointers.size === 0) this.#gestureMoved = false;
    this.#pointers.set(event.pointerId, { x: event.global.x, y: event.global.y });
  };
  readonly #handlePointerMove = (event: FederatedPointerEvent): void => {
    const previous = this.#pointers.get(event.pointerId);
    if (previous === undefined) return;
    const next = { x: event.global.x, y: event.global.y };
    const other = [...this.#pointers.entries()].find(([id]) => id !== event.pointerId)?.[1];
    this.#pointers.set(event.pointerId, next);
    if (Math.abs(next.x - previous.x) + Math.abs(next.y - previous.y) < 2) return;
    this.#gestureMoved = true;
    if (other === undefined) this.#camera.panBy(next.x - previous.x, next.y - previous.y);
    else {
      const before = Math.hypot(previous.x - other.x, previous.y - other.y);
      const after = Math.hypot(next.x - other.x, next.y - other.y);
      if (before > 1)
        this.#callbacks.onZoomChange(
          this.#camera.zoomBy(after / before, (next.x + other.x) / 2, (next.y + other.y) / 2).scale,
        );
    }
    this.#scheduleRender();
  };
  readonly #handlePointerUp = (event: FederatedPointerEvent): void => {
    this.#pointers.delete(event.pointerId);
  };

  #renderNow(animation = false): void {
    const app = this.#app;
    const objects = this.#objectsLayer;
    const world = this.#worldLayer;
    if (app === null || objects === null || world === null) return;
    const started = performance.now();
    const camera = this.#camera.snapshot();
    world.position.set(camera.x, camera.y);
    world.scale.set(camera.scale);
    const detail: LobbySceneDetail = sceneDetailForZoom(camera.scale);
    const viewport = this.#camera.viewport();
    const visibleAgents = new Set(
      visibleLobbyNodes(this.#projection, {
        ...viewport,
        x: viewport.x - 120,
        y: viewport.y - 120,
        width: viewport.width + 240,
        height: viewport.height + 240,
      }).map((node) => node.agentId),
    );
    const visible = sceneCharacters(this.#projection, this.#labels.self).filter(
      (node) => node.kind === 'human' || visibleAgents.has(node.characterId),
    );
    const frameCharacters: SceneFrame['characters'][number][] = [];
    const visibleIds = new Set(visible.map((node) => node.characterId));
    for (const [id, stored] of this.#views) {
      if (!visibleIds.has(id)) {
        stored.view.destroy();
        this.#views.delete(id);
      }
    }
    for (const node of visible) {
      const selected = node.characterId === this.#projection.selectedAgentId;
      const signature = [
        node.displayName,
        node.status,
        node.kind,
        node.radius,
        detail,
        selected,
      ].join(':');
      let stored = this.#views.get(node.characterId);
      if (stored?.signature !== signature) {
        stored?.view.destroy();
        const view = createAgentNodeView(this.#pixi, {
          detail,
          node,
          onSelect: (id) => {
            if (node.kind === 'human') {
              if (!this.#gestureMoved) this.#callbacks.onSelectHuman?.(node.matrixUserId);
            } else this.#selectAgent(id);
          },
          selected,
        });
        objects.addChild(view.container);
        stored = { signature, view };
        this.#views.set(node.characterId, stored);
      }
      const pose = characterPose(node, this.#elapsedSeconds, !this.#motion.matches && !selected);
      stored.view.animate(pose);
      frameCharacters.push({
        characterId: node.characterId,
        x: camera.x + pose.x * camera.scale,
        y: camera.y + (pose.y - 95 * Math.max(0.83, node.radius / 27)) * camera.scale,
      });
    }
    if (!animation)
      this.#host.dataset.agentRoomUpdateMilliseconds = String(performance.now() - started);
    app.render();
    this.#callbacks.onFrame?.({
      width: app.screen.width,
      height: app.screen.height,
      characters: frameCharacters,
    });
    if (!animation) {
      this.#host.dataset.agentRoomRenderedNodes = String(visibleAgents.size);
      this.#host.dataset.agentRoomTextureCount = String(
        (app.renderer as TextureAwareRenderer).texture?.managedTextures?.length ?? 0,
      );
      this.#host.dataset.agentRoomRenderMilliseconds = String(performance.now() - started);
      this.#host.dataset.agentRoomRenderSequence = String(
        Number(this.#host.dataset.agentRoomRenderSequence ?? '0') + 1,
      );
    }
  }

  #scheduleRender(): void {
    if (this.#frame !== null || this.#destroyed) return;
    this.#frame = window.requestAnimationFrame(() => {
      this.#frame = null;
      this.#renderNow();
    });
  }
}
