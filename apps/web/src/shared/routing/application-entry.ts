export type ApplicationEntryLocation = {
  readonly hash: string;
  readonly pathname: string;
  readonly search: string;
};

const desktopBuildMode = 'desktop';
const desktopEntryPath = '/connect';

/**
 * Keeps the public website on its marketing home while giving the packaged
 * desktop application a product entry point. Deep links and authentication
 * callbacks are deliberately left untouched.
 */
export function resolveApplicationEntry(
  mode: string,
  location: ApplicationEntryLocation,
): string | null {
  if (
    mode !== desktopBuildMode ||
    location.pathname !== '/' ||
    location.search.length > 0 ||
    location.hash.length > 0
  ) {
    return null;
  }
  return desktopEntryPath;
}

export function applyApplicationEntry(
  mode: string = import.meta.env.MODE,
  location: ApplicationEntryLocation = window.location,
  history: Pick<History, 'replaceState' | 'state'> = window.history,
): void {
  const target = resolveApplicationEntry(mode, location);
  if (target !== null) {
    history.replaceState(history.state, '', target);
  }
}
