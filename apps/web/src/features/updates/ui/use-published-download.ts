import { useEffect, useState } from 'react';
import { readRuntimeManifest } from '../domain/runtime-manifest';

/** A cached page checks today's distribution link without requiring a page update. */
export function usePublishedDownload(fallback: string | null): string | null {
  const [latest, setLatest] = useState<{ readonly url: string | null } | null>(null);
  useEffect(() => {
    const abort = new AbortController();
    void readRuntimeManifest(abort.signal)
      .then((manifest) => {
        if (!abort.signal.aborted) setLatest({ url: manifest.windowsDownloadUrl });
      })
      .catch(() => {
        if (!abort.signal.aborted) setLatest(null);
      });
    return () => {
      abort.abort();
    };
  }, []);
  // When offline, release pages resolve the available installer instead of pinning an obsolete file.
  if (latest !== null) return latest.url;
  if (fallback?.startsWith('https://github.com/rainyflash/agent-room/releases/'))
    return 'https://github.com/rainyflash/agent-room/releases';
  return fallback;
}
