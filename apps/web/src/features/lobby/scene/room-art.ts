import {
  projectFloorPoint,
  roomFloor,
  roomFurnishings,
  type FloorPoint,
  type RoomFurnishing,
} from '@/features/lobby/domain/room-floor';

export type SceneShape =
  | {
      readonly kind: 'polygon';
      readonly points: readonly number[];
      readonly fill: string;
      readonly stroke?: string;
    }
  | {
      readonly kind: 'ellipse';
      readonly x: number;
      readonly y: number;
      readonly rx: number;
      readonly ry: number;
      readonly fill: string;
      readonly stroke?: string;
    }
  | {
      readonly kind: 'rect';
      readonly x: number;
      readonly y: number;
      readonly width: number;
      readonly height: number;
      readonly radius: number;
      readonly fill: string;
      readonly stroke?: string;
    };

export type RoomPropArt = { readonly depth: number; readonly shapes: readonly SceneShape[] };

const point = (x: number, y: number, elevation = 0): FloorPoint =>
  projectFloorPoint({ x, y }, elevation);

function polygon(vertices: readonly FloorPoint[], fill: string, stroke?: string): SceneShape {
  return {
    kind: 'polygon',
    points: vertices.flatMap((vertex) => [vertex.x, vertex.y]),
    fill,
    ...(stroke === undefined ? {} : { stroke }),
  };
}

function tile(
  x: number,
  y: number,
  width: number,
  depth: number,
  elevation: number,
  fill: string,
  stroke?: string,
): SceneShape {
  return polygon(
    [
      point(x, y, elevation),
      point(x + width, y, elevation),
      point(x + width, y + depth, elevation),
      point(x, y + depth, elevation),
    ],
    fill,
    stroke,
  );
}

function block(
  x: number,
  y: number,
  width: number,
  depth: number,
  height: number,
  top: string,
  left: string,
  right: string,
  base = 0,
): SceneShape[] {
  return [
    polygon(
      [
        point(x, y + depth, base),
        point(x + width, y + depth, base),
        point(x + width, y + depth, base + height),
        point(x, y + depth, base + height),
      ],
      left,
    ),
    polygon(
      [
        point(x + width, y, base),
        point(x + width, y + depth, base),
        point(x + width, y + depth, base + height),
        point(x + width, y, base + height),
      ],
      right,
    ),
    tile(x, y, width, depth, base + height, top),
  ];
}

export function roomGroundArt(): readonly SceneShape[] {
  const { width, depth } = roomFloor;
  const shapes: SceneShape[] = [
    { kind: 'ellipse', x: 1390, y: 935, rx: 980, ry: 335, fill: '#d0d7c5' },
    ...block(0, 0, width, depth, 30, '#e0c9a6', '#ab916d', '#b79d78', -30),
  ];
  for (let x = 0; x < width; x += 100) {
    for (let y = 0; y < depth; y += 200) {
      const offset = (Math.floor(x / 100) + Math.floor(y / 200)) % 3;
      shapes.push(
        tile(x + 1, y + 1, 98, 198, 1, ['#dfc8a5', '#e4cfad', '#dcc39f'][offset] ?? '#dfc8a5'),
      );
    }
  }
  shapes.push(
    ...block(-18, -18, width + 36, 18, 155, '#f8f3e6', '#d6dfce', '#a7bba9'),
    ...block(-18, 0, 18, depth, 155, '#f8f3e6', '#b2c4b5', '#ced8c9'),
    tile(125, 130, 970, 485, 3, '#a7b9a1', '#8caa8c'),
    tile(1140, 235, 455, 420, 3, '#aba4bb', '#9389a5'),
    tile(190, 650, 800, 285, 3, '#c9ad89', '#b59771'),
    tile(205, 665, 770, 255, 4, '#eadac0'),
  );
  for (const x of [450, 630, 810, 990]) {
    shapes.push(
      polygon(
        [point(x, 0, 35), point(x + 135, 0, 35), point(x + 135, 0, 132), point(x, 0, 132)],
        '#eef6e9',
        '#8da994',
      ),
    );
    shapes.push(
      polygon(
        [point(x + 5, 0, 40), point(x + 125, 0, 125), point(x + 90, 0, 125), point(x + 5, 0, 62)],
        '#d5e8da',
      ),
    );
  }
  return shapes;
}

const builders: Readonly<Record<RoomFurnishing['kind'], (item: RoomFurnishing) => SceneShape[]>> = {
  desk: (item) => {
    const { x, y, width, depth } = item;
    const shapes = [
      ...block(x + 10, y + 10, 12, depth - 14, 48, '#697a66', '#586852', '#45573f'),
      ...block(x + width - 22, y + 10, 12, depth - 14, 48, '#697a66', '#586852', '#45573f'),
      ...block(x, y, width, depth, 10, '#f3e4c8', '#c5b38c', '#ddcba7', 48),
      ...block(x + 52, y + 22, 50, 12, 5, '#617c75', '#456158', '#789285', 58),
      ...block(x + 40, y + 17, 76, 8, 49, '#718e80', '#304b46', '#526e5f', 63),
      polygon(
        [
          point(x + 46, y + 25, 70),
          point(x + 110, y + 25, 70),
          point(x + 110, y + 25, 105),
          point(x + 46, y + 25, 105),
        ],
        '#9bcfbc',
      ),
      tile(x + 49, y + 47, 65, 25, 60, '#a9b7a3'),
      ...block(x + 132, y + 40, 13, 13, 18, '#dcb67e', '#bb9662', '#d1a871', 58),
    ];
    return shapes;
  },
  table: ({ x, y, width, depth }) => [
    ...block(x + 25, y + 20, width - 50, depth - 40, 42, '#a57856', '#916344', '#795138'),
    ...block(x, y, width, depth, 13, '#cfa580', '#a97f59', '#b88c64', 42),
    tile(x + 25, y + 22, 48, 38, 56, '#faf2df'),
    tile(x + 31, y + 26, 29, 6, 57, '#93afa4'),
    ...block(x + width - 42, y + depth - 42, 19, 19, 17, '#e8d5bb', '#c7b08e', '#bba481', 56),
  ],
  sofa: ({ x, y, width, depth }) => [
    ...block(x, y, width, depth, 32, '#799882', '#526e5d', '#64816d'),
    ...block(x, y, width, 20, 42, '#9ab29a', '#719175', '#7e9d82', 30),
    ...block(x, y + 20, 22, depth - 20, 30, '#9ab29a', '#719175', '#7e9d82', 30),
    ...block(x + width - 22, y + 20, 22, depth - 20, 30, '#9ab29a', '#719175', '#7e9d82', 30),
    ...block(x + 30, y + 23, 88, depth - 28, 9, '#a7bca1', '#819b7f', '#90aa8c', 32),
    ...block(x + 125, y + 23, 88, depth - 28, 9, '#adc1a7', '#819b7f', '#90aa8c', 32),
  ],
  plant: ({ x, y }) => {
    const center = point(x + 30, y + 30);
    return [
      ...block(x + 12, y + 12, 36, 36, 35, '#dca27d', '#ba7957', '#c68863'),
      { kind: 'ellipse', x: center.x, y: center.y - 34, rx: 22, ry: 10, fill: '#536647' },
      ...[-20, 0, 20].map((dx) => ({
        kind: 'ellipse' as const,
        x: center.x + dx,
        y: center.y - 68 - (dx === 0 ? 16 : 0),
        rx: 20,
        ry: 36,
        fill: dx === 0 ? '#729460' : '#527b50',
      })),
    ];
  },
  shelf: ({ x, y, width, depth }) => {
    const shapes = block(x, y, width, depth, 118, '#c9a780', '#927354', '#ad8b65');
    for (let index = 0; index < 14; index += 1) {
      shapes.push(
        ...block(
          x + 10 + index * 18,
          y + depth,
          12,
          5,
          32 + (index % 3) * 4,
          '#ece3ce',
          ['#748f7c', '#aa8276', '#e0ba79', '#839caa'][index % 4] ?? '#748f7c',
          '#8e9a80',
          60,
        ),
      );
    }
    return shapes;
  },
  server: ({ x, y, width, depth }) => {
    const shapes = block(x, y, width, depth, 123, '#6d7a7e', '#394e53', '#526669');
    for (const height of [26, 56, 86]) {
      shapes.push(
        polygon(
          [
            point(x + 12, y + depth + 1, height),
            point(x + width - 12, y + depth + 1, height),
            point(x + width - 12, y + depth + 1, height + 18),
            point(x + 12, y + depth + 1, height + 18),
          ],
          '#263f43',
        ),
      );
      const led = point(x + 23, y + depth + 1, height + 9);
      shapes.push({ kind: 'ellipse', x: led.x, y: led.y, rx: 3, ry: 3, fill: '#bdde9c' });
    }
    return shapes;
  },
};

export function roomPropsArt(): readonly RoomPropArt[] {
  return roomFurnishings.map((item) => ({
    depth: point(item.x + item.width / 2, item.y + item.depth).y,
    shapes: builders[item.kind](item),
  }));
}

export const roomPlaques = [
  { id: 'active' as const, ...projectFloorPoint({ x: 490, y: 85 }) },
  { id: 'attention' as const, ...projectFloorPoint({ x: 1290, y: 205 }) },
  { id: 'available' as const, ...projectFloorPoint({ x: 710, y: 925 }) },
];
