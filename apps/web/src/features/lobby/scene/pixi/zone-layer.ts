import type { Container } from 'pixi.js';
import type { LobbySceneProjection, LobbyZoneId } from '@/features/lobby/domain/scene-projection';
import { roomGroundArt, roomPropsArt, roomPlaques } from '../room-art';
import { shapeGraphics } from './shape-graphics';

type PixiModule = typeof import('pixi.js');

export function createZoneLayer(
  pixi: PixiModule,
  _projection: LobbySceneProjection,
  labels: Readonly<Record<LobbyZoneId, string>>,
): Container {
  const layer = new pixi.Container();
  layer.eventMode = 'none';
  layer.addChild(shapeGraphics(pixi, roomGroundArt()));
  for (const plaque of roomPlaques) {
    const label = new pixi.Text({
      text: labels[plaque.id],
      anchor: 0.5,
      style: {
        fontFamily: 'Instrument Sans, Noto Sans SC, sans-serif',
        fontSize: 22,
        fontWeight: '600',
        fill: '#526b55',
        letterSpacing: 2,
      },
    });
    label.position.set(plaque.x, plaque.y);
    layer.addChild(label);
  }
  // 地面与标牌不随角色变化，按世界尺寸缓存，避免每帧重复提交复杂几何。
  layer.cacheAsTexture({ resolution: 1, antialias: true });
  return layer;
}

export function createRoomProps(pixi: PixiModule): readonly Container[] {
  return roomPropsArt().map((prop) => {
    const container = new pixi.Container();
    container.eventMode = 'none';
    container.zIndex = prop.depth;
    container.addChild(shapeGraphics(pixi, prop.shapes));
    container.cacheAsTexture({ resolution: 2, antialias: true });
    return container;
  });
}
