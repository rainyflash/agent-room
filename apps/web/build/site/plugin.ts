import { fileURLToPath } from 'node:url';
import { loadEnv, type Plugin } from 'vite';

import { applicationVersion } from '../runtime-manifest.js';
import { siteFiles, writeSite, type SiteSources } from './generate.js';
import { REPOSITORY_URL } from './html.js';

const LATEST_RELEASE = `${REPOSITORY_URL}/releases/latest`;

function local(relative: string): string {
  return fileURLToPath(new URL(relative, import.meta.url));
}

export const siteSources: SiteSources = {
  siteDir: local('../../site/'),
  publicDir: local('../../public/'),
  // 字体是 ui-system 的依赖；网关镜像按 `--filter @agent-room/web...` 安装，也带着它。
  fontFile: local(
    '../../../../packages/ui-system/node_modules/@fontsource-variable/fredoka/files/fredoka-latin-wght-normal.woff2',
  ),
};

function downloadUrl(value: string | undefined): string {
  return value === undefined || value === '' ? LATEST_RELEASE : value;
}

/**
 * agentroom.chat 的静态网站（specs/site-discovery/design.md）：应用构建完以后写到 `dist-site/`，
 * 网关镜像拷到 `/srv/site`。不进应用的 `dist/`，离线缓存不会带上它。桌面端和验收用的构建不生成。
 */
export function staticSite(mode: string): Plugin {
  const env = loadEnv(mode, process.cwd(), 'VITE_AGENT_ROOM_');
  return {
    name: 'agent-room-static-site',
    apply: 'build',
    async closeBundle() {
      if (mode !== 'production') return;
      const files = await siteFiles(siteSources, {
        version: applicationVersion,
        builtOn: new Date().toISOString().slice(0, 10),
        downloads: {
          windows: downloadUrl(
            process.env.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL ??
              env.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL,
          ),
          macos: downloadUrl(
            process.env.VITE_AGENT_ROOM_MACOS_DOWNLOAD_URL ??
              env.VITE_AGENT_ROOM_MACOS_DOWNLOAD_URL,
          ),
        },
        guides: [],
      });
      await writeSite(local('../../dist-site/'), files);
    },
  };
}
