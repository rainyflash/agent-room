// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RuntimeCompatibilityProvider } from './runtime-compatibility-provider';
import {
  useRuntimeCompatibility,
  type RuntimeCompatibility,
} from './runtime-compatibility-context';
import { currentWriteContract } from '../domain/runtime-manifest';
import { registerUpdateGuard } from '../application/update-readiness';

const sw = vi.hoisted(() => ({ update: vi.fn(() => Promise.resolve()) }));
vi.mock('virtual:pwa-register/react', () => ({
  useRegisterSW: () => ({ needRefresh: [true], updateServiceWorker: sw.update }),
}));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  sw.update.mockClear();
});
let runtime: RuntimeCompatibility;
function Probe() {
  runtime = useRuntimeCompatibility();
  return <output>{String(runtime.writes.allowed)}</output>;
}
function renderWith(contract: string) {
  vi.stubGlobal(
    'fetch',
    vi.fn(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({ schema: 1, writeContract: contract, windowsDownloadUrl: null }),
        ),
      ),
    ),
  );
  render(
    <RuntimeCompatibilityProvider>
      <Probe />
    </RuntimeCompatibilityProvider>,
  );
}
describe('update lifecycle', () => {
  it('fetches fresh compatibility metadata and keeps compatible updates writable', async () => {
    renderWith(currentWriteContract);
    await waitFor(() => {
      expect(screen.getByRole('status').textContent).toBe('true');
    });
    expect(fetch).toHaveBeenCalledWith(
      '/runtime-manifest.json',
      expect.objectContaining({ cache: 'no-store' }),
    );
    await runtime.applyUpdate();
    expect(sw.update).toHaveBeenCalledExactlyOnceWith(true);
  });
  it('blocks changed or unavailable contracts and refuses to discard an unsaved draft', async () => {
    renderWith('a'.repeat(64));
    await waitFor(() => {
      expect(screen.getByRole('status').textContent).toBe('false');
    });
    const detach = registerUpdateGuard(() => false);
    try {
      await expect(runtime.applyUpdate()).rejects.toThrow('update.draft_unsaved');
    } finally {
      detach();
    }
    expect(sw.update).not.toHaveBeenCalled();
  });
});
