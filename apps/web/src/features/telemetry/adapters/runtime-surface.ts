import type { FrontendSurface } from '@/features/telemetry/domain/frontend-metric';

export function resolveFrontendSurface(): FrontendSurface {
  return Object.hasOwn(window, '__TAURI_INTERNALS__') ? 'desktop' : 'web';
}
