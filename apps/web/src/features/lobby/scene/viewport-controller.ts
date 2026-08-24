import type { LobbyViewport, LobbyWorld } from '@/features/lobby/domain/scene-projection';

export type CameraSnapshot = {
  readonly scale: number;
  readonly x: number;
  readonly y: number;
};

export class ViewportController {
  readonly #maximumScale: number;
  readonly #minimumScale: number;
  readonly #padding: number;
  readonly #world: LobbyWorld;
  #initialized = false;
  #scale = 1;
  #screenHeight = 1;
  #screenWidth = 1;
  #x = 0;
  #y = 0;

  constructor(
    world: LobbyWorld,
    options: {
      readonly maximumScale?: number;
      readonly minimumScale?: number;
      readonly padding?: number;
    } = {},
  ) {
    this.#world = world;
    this.#maximumScale = options.maximumScale ?? 1.8;
    this.#minimumScale = options.minimumScale ?? 0.25;
    this.#padding = options.padding ?? 48;
  }

  panBy(deltaX: number, deltaY: number): CameraSnapshot {
    this.#x += finiteOrZero(deltaX);
    this.#y += finiteOrZero(deltaY);
    this.#clampPosition();
    return this.snapshot();
  }

  reset(): CameraSnapshot {
    const availableWidth = Math.max(1, this.#screenWidth - this.#padding * 2);
    const availableHeight = Math.max(1, this.#screenHeight - this.#padding * 2);
    this.#scale = clamp(
      Math.min(availableWidth / this.#world.width, availableHeight / this.#world.height),
      this.#minimumScale,
      this.#maximumScale,
    );
    this.#x = (this.#screenWidth - this.#world.width * this.#scale) / 2;
    this.#y = (this.#screenHeight - this.#world.height * this.#scale) / 2;
    this.#initialized = true;
    return this.snapshot();
  }

  resize(width: number, height: number): CameraSnapshot {
    const center = this.#initialized
      ? {
          x: (this.#screenWidth / 2 - this.#x) / this.#scale,
          y: (this.#screenHeight / 2 - this.#y) / this.#scale,
        }
      : null;
    this.#screenWidth = validExtent(width);
    this.#screenHeight = validExtent(height);
    if (center === null) {
      return this.reset();
    }
    this.#x = this.#screenWidth / 2 - center.x * this.#scale;
    this.#y = this.#screenHeight / 2 - center.y * this.#scale;
    this.#clampPosition();
    return this.snapshot();
  }

  snapshot(): CameraSnapshot {
    return Object.freeze({ scale: this.#scale, x: this.#x, y: this.#y });
  }

  viewport(): LobbyViewport {
    return Object.freeze({
      height: this.#screenHeight / this.#scale,
      width: this.#screenWidth / this.#scale,
      x: -this.#x / this.#scale,
      y: -this.#y / this.#scale,
      zoom: this.#scale,
    });
  }

  zoomBy(factor: number, screenX = this.#screenWidth / 2, screenY = this.#screenHeight / 2) {
    const nextScale = clamp(
      this.#scale * (Number.isFinite(factor) && factor > 0 ? factor : 1),
      this.#minimumScale,
      this.#maximumScale,
    );
    const anchorX = finiteOrZero(screenX);
    const anchorY = finiteOrZero(screenY);
    const worldX = (anchorX - this.#x) / this.#scale;
    const worldY = (anchorY - this.#y) / this.#scale;
    this.#scale = nextScale;
    this.#x = anchorX - worldX * nextScale;
    this.#y = anchorY - worldY * nextScale;
    this.#clampPosition();
    return this.snapshot();
  }

  #clampPosition(): void {
    this.#x = clampAxis(this.#x, this.#world.width * this.#scale, this.#screenWidth);
    this.#y = clampAxis(this.#y, this.#world.height * this.#scale, this.#screenHeight);
  }
}

function clampAxis(position: number, contentExtent: number, screenExtent: number): number {
  if (contentExtent <= screenExtent) {
    return (screenExtent - contentExtent) / 2;
  }
  return clamp(position, screenExtent - contentExtent, 0);
}

function validExtent(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 1;
}

function finiteOrZero(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}
