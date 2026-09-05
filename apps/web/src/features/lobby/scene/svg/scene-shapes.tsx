import type { SceneShape } from '../room-art';

export function SceneShapes({ shapes }: { readonly shapes: readonly SceneShape[] }) {
  return shapes.map((shape, index) => {
    const style = { fill: shape.fill, stroke: shape.stroke, strokeWidth: 1.2 };
    switch (shape.kind) {
      case 'polygon':
        return <polygon key={index} points={shape.points.join(' ')} style={style} />;
      case 'ellipse':
        return (
          <ellipse
            key={index}
            cx={shape.x}
            cy={shape.y}
            rx={shape.rx}
            ry={shape.ry}
            style={style}
          />
        );
      case 'rect':
        return (
          <rect
            key={index}
            x={shape.x}
            y={shape.y}
            width={shape.width}
            height={shape.height}
            rx={shape.radius}
            style={style}
          />
        );
    }
  });
}
