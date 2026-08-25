import { useSyncExternalStore } from 'react';

import {
  assessListModeRequirement,
  type ListModeRequirement,
} from '@/features/lobby/domain/rendering-capability';

const compactQuery = '(max-width: 767px)';
const forcedColorsQuery = '(forced-colors: active)';

export function useListModeRequirement(): ListModeRequirement {
  return useSyncExternalStore(subscribe, snapshot, serverSnapshot);
}

function subscribe(listener: () => void): () => void {
  if (typeof window.matchMedia !== 'function') {
    return () => undefined;
  }
  const queries = [window.matchMedia(compactQuery), window.matchMedia(forcedColorsQuery)];
  for (const query of queries) {
    query.addEventListener('change', listener);
  }
  return () => {
    for (const query of queries) {
      query.removeEventListener('change', listener);
    }
  };
}

function snapshot(): ListModeRequirement {
  const memory = (window.navigator as Navigator & { readonly deviceMemory?: number }).deviceMemory;
  return assessListModeRequirement({
    compactViewport:
      typeof window.matchMedia === 'function' && window.matchMedia(compactQuery).matches,
    deviceMemoryGiB: typeof memory === 'number' ? memory : null,
    forcedColors:
      typeof window.matchMedia === 'function' && window.matchMedia(forcedColorsQuery).matches,
    hardwareConcurrency:
      window.navigator.hardwareConcurrency > 0 ? window.navigator.hardwareConcurrency : null,
  });
}

function serverSnapshot(): ListModeRequirement {
  return null;
}
