import type { Container } from 'pixi.js';

import type { LobbySceneProjection, LobbyZoneId } from '@/features/lobby/domain/scene-projection';

type PixiModule = typeof import('pixi.js');

const ZONE_COLOR: Readonly<Record<LobbyZoneId, number>> = Object.freeze({
  active: 0x9f_e8_70,
  attention: 0xff_6b_3d,
  available: 0x66_c9_d8,
});

export function createZoneLayer(
  pixi: PixiModule,
  projection: LobbySceneProjection,
  labels: Readonly<Record<LobbyZoneId, string>>,
): Container {
  const layer = new pixi.Container();
  layer.eventMode = 'none';
  for (const zone of projection.zones) {
    layer.addChild(
      new pixi.Graphics()
        .roundRect(zone.x, zone.y, zone.width, zone.height, 34)
        .fill({ alpha: 0.018, color: ZONE_COLOR[zone.id] })
        .stroke({ alpha: 0.18, color: ZONE_COLOR[zone.id], width: 1 }),
    );
    const label = new pixi.Text({
      style: {
        fill: ZONE_COLOR[zone.id],
        fontFamily: 'IBM Plex Mono, monospace',
        fontSize: 13,
        fontWeight: '600',
        letterSpacing: 1.4,
      },
      text: labels[zone.id].toLocaleUpperCase(),
    });
    label.position.set(zone.x + 24, zone.y + 20);
    layer.addChild(label);
  }
  return layer;
}
