import { useEffect, useState } from 'react';
import {
  detectVisitorPlatform,
  type VisitorPlatform,
} from '@/features/landing/domain/visitor-platform';
import { readRuntimeManifest } from '../domain/runtime-manifest';

const RELEASES_URL = 'https://github.com/rainyflash/agent-room/releases';

export type DownloadConfiguration = {
  readonly windowsDownloadUrl: string | null;
  readonly macosDownloadUrl: string | null;
};

export type PublishedDownload = {
  /** 访客这个系统的安装包；还没有发行版时是 null，页面据此给出替代路径。 */
  readonly url: string | null;
  readonly platform: VisitorPlatform;
};

type PlatformDownloads = {
  readonly windows: string | null;
  readonly macos: string | null;
};

function downloadFor(downloads: PlatformDownloads, platform: VisitorPlatform): string | null {
  if (platform === 'windows') return downloads.windows;
  if (platform === 'macos') return downloads.macos;
  return null;
}

/** A cached page checks today's distribution link without requiring a page update. */
export function usePublishedDownload(config: DownloadConfiguration): PublishedDownload {
  const [latest, setLatest] = useState<PlatformDownloads | null>(null);
  useEffect(() => {
    const abort = new AbortController();
    void readRuntimeManifest(abort.signal)
      .then((manifest) => {
        if (!abort.signal.aborted)
          setLatest({
            windows: manifest.windowsDownloadUrl,
            macos: manifest.macosDownloadUrl ?? null,
          });
      })
      .catch(() => {
        if (!abort.signal.aborted) setLatest(null);
      });
    return () => {
      abort.abort();
    };
  }, []);
  const platform = detectVisitorPlatform(window.navigator.userAgent);
  if (latest !== null) return { url: downloadFor(latest, platform), platform };
  const fallback = downloadFor(
    { windows: config.windowsDownloadUrl, macos: config.macosDownloadUrl },
    platform,
  );
  // When offline, release pages resolve the available installer instead of pinning an obsolete file.
  if (fallback?.startsWith(`${RELEASES_URL}/`)) return { url: RELEASES_URL, platform };
  return { url: fallback, platform };
}
