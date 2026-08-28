/**
 * 在保留部署路径前缀的前提下解析 Control Plane 端点。
 *
 * `new URL('/path', base)` 会静默丢弃 `base` 的路径；浏览器同源 BFF
 * 依赖固定前缀，因此这里集中维护该不变量。
 */
export function controlPlaneEndpoint(baseUrl: string, endpointPath: string): URL {
  if (!endpointPath.startsWith('/') || endpointPath.startsWith('//')) {
    throw new TypeError('Control Plane 端点必须是单斜杠开头的站内路径。');
  }

  const base = new URL(baseUrl);
  if (
    !['http:', 'https:'].includes(base.protocol) ||
    base.username.length > 0 ||
    base.password.length > 0 ||
    base.search.length > 0 ||
    base.hash.length > 0
  ) {
    throw new TypeError('Control Plane 基址必须是无凭据、查询和片段的 HTTP(S) URL。');
  }
  const prefix = base.pathname === '/' ? '/' : `${base.pathname.replace(/\/+$/u, '')}/`;
  base.pathname = prefix;
  base.search = '';
  base.hash = '';

  const endpoint = new URL(endpointPath.slice(1), base);
  if (endpoint.origin !== base.origin || !endpoint.pathname.startsWith(prefix)) {
    throw new TypeError('Control Plane 端点不能逃逸部署路径前缀。');
  }
  return endpoint;
}
