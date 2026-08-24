import { useSyncExternalStore } from 'react';

const compactLobbyQuery = '(max-width: 767px)';

export function useCompactLobby(): boolean {
  return useSyncExternalStore(subscribe, snapshot, serverSnapshot);
}

function subscribe(listener: () => void): () => void {
  const mediaQuery = window.matchMedia(compactLobbyQuery);
  mediaQuery.addEventListener('change', listener);
  return () => {
    mediaQuery.removeEventListener('change', listener);
  };
}

function snapshot(): boolean {
  return window.matchMedia(compactLobbyQuery).matches;
}

function serverSnapshot(): boolean {
  return false;
}
