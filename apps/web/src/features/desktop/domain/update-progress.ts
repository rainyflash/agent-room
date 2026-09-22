import type { TFunction } from 'i18next';
import type { ReleaseUpdateProgress } from './desktop-runtime';

/** Button text while an update installs: a percentage while downloading, then "installing". */
export function updateProgressLabel(t: TFunction, progress: ReleaseUpdateProgress | null): string {
  if (progress === null || progress.phase === 'downloading') {
    const percent =
      progress === null || progress.totalBytes === 0
        ? 0
        : Math.min(100, Math.floor((progress.downloadedBytes * 100) / progress.totalBytes));
    return t('desktop.update.downloading', { percent });
  }
  return t('desktop.update.installing');
}
