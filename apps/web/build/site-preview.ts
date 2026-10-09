import { loadEnv, type Plugin } from 'vite';

export const SOCIAL_PREVIEW_PATH = '/social-preview.png';

/**
 * 把链接发到别处时的预览图（`og:image`）。它必须是绝对地址，而网页不知道自己部署在哪个域名下：
 * 生产和自建都把控制面挂在网页自己的域名下（`https://<网页域名>/_agent-room/api`），就用那个 Origin。
 * 控制面在别的域名（桌面端、本地开发）时不写预览图，链接照样有标题和简介。
 */
export function sitePreview(mode: string): Plugin {
  const env = loadEnv(mode, process.cwd(), 'VITE_AGENT_ROOM_');
  const origin = siteOrigin(
    process.env.VITE_AGENT_ROOM_CONTROL_PLANE_URL ?? env.VITE_AGENT_ROOM_CONTROL_PLANE_URL,
  );
  return {
    name: 'agent-room-site-preview',
    transformIndexHtml() {
      if (origin === null) return [];
      const image = {
        'og:image': `${origin}${SOCIAL_PREVIEW_PATH}`,
        'og:image:width': '1280',
        'og:image:height': '640',
        'og:image:alt': 'Agent Room: people and AI agents together in one room',
      };
      return Object.entries(image).map(([property, content]) => ({
        tag: 'meta',
        attrs: { property, content },
        injectTo: 'head' as const,
      }));
    },
  };
}

/** 控制面挂在网页域名下的某个路径时，那个 https Origin 就是网站；否则不知道。 */
export function siteOrigin(controlPlaneUrl: string | undefined): string | null {
  if (controlPlaneUrl === undefined || !URL.canParse(controlPlaneUrl)) return null;
  const url = new URL(controlPlaneUrl);
  if (url.protocol !== 'https:' || url.pathname.replace(/\/+$/u, '') === '') return null;
  return url.origin;
}
