import {
  applicationVersion,
  currentWriteContract,
  readRuntimeManifest,
} from '../domain/runtime-manifest';
import { prepareForUpdate } from '../application/update-readiness';

type InstallingWorker = Pick<ServiceWorker, 'state' | 'addEventListener' | 'removeEventListener'>;

/** update() can resolve before the new worker has finished caching the application. */
export function waitForWorkerInstallation(worker: InstallingWorker | null): Promise<void> {
  if (worker === null) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const finish = (failure?: Error) => {
      clearTimeout(timeout);
      worker.removeEventListener('statechange', changed);
      if (failure === undefined) resolve();
      else reject(failure);
    };
    const changed = () => {
      if (['installed', 'activating', 'activated'].includes(worker.state)) finish();
      else if (worker.state === 'redundant') finish(new Error('update.install_failed'));
    };
    const timeout = setTimeout(() => {
      finish(new Error('update.install_timeout'));
    }, 30_000);
    worker.addEventListener('statechange', changed);
    changed();
  });
}

async function refreshWorker() {
  const registration =
    'serviceWorker' in navigator ? await navigator.serviceWorker.getRegistration() : undefined;
  if (registration !== undefined) {
    await registration.update();
    await waitForWorkerInstallation(registration.installing);
  }
  return registration;
}

export async function checkWebApplicationUpdate(): Promise<boolean> {
  const manifest = await readRuntimeManifest();
  const registration = await refreshWorker();
  return (
    registration?.waiting != null ||
    (manifest.version !== undefined && manifest.version !== applicationVersion) ||
    manifest.writeContract !== currentWriteContract
  );
}

export async function applyWebApplicationUpdate(
  activateWorker: () => Promise<void>,
): Promise<void> {
  if (!prepareForUpdate()) throw new Error('update.draft_unsaved');
  const registration = await refreshWorker();
  // A draft may have changed while the new assets were being cached.
  if (!prepareForUpdate()) throw new Error('update.draft_unsaved');
  if (registration?.waiting != null) await activateWorker();
  else window.location.reload();
}
