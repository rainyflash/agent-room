import { useMemo } from 'react';
import { encode } from 'uqr';

/**
 * 链接的二维码，在本机画，不经过别的服务。深浅主题下都是白底黑块：扫码的软件认这个。
 */
export function QrCode({ label, value }: { readonly label: string; readonly value: string }) {
  const { path, size } = useMemo(() => {
    const qr = encode(value, { border: 2, ecc: 'M' });
    let d = '';
    qr.data.forEach((row, y) => {
      row.forEach((dark, x) => {
        if (dark) d += `M${String(x)} ${String(y)}h1v1h-1z`;
      });
    });
    return { path: d, size: qr.size };
  }, [value]);
  return (
    <svg
      aria-label={label}
      className="qr-code"
      role="img"
      shapeRendering="crispEdges"
      viewBox={`0 0 ${String(size)} ${String(size)}`}
    >
      <rect fill="#ffffff" height={size} width={size} />
      <path d={path} fill="#000000" />
    </svg>
  );
}
