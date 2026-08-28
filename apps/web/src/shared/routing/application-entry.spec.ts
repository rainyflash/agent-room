import { describe, expect, it, vi } from 'vitest';

import { applyApplicationEntry, resolveApplicationEntry } from '@/shared/routing/application-entry';

describe('application entry routing', () => {
  it('opens the packaged desktop product at the connection flow', () => {
    expect(
      resolveApplicationEntry('desktop', {
        hash: '',
        pathname: '/',
        search: '',
      }),
    ).toBe('/connect');
  });

  it('keeps the public website on the marketing home', () => {
    expect(
      resolveApplicationEntry('production', {
        hash: '',
        pathname: '/',
        search: '',
      }),
    ).toBeNull();
  });

  it('does not overwrite a product route or callback context', () => {
    expect(
      resolveApplicationEntry('desktop', {
        hash: '',
        pathname: '/connect',
        search: '?code=single-use',
      }),
    ).toBeNull();
    expect(
      resolveApplicationEntry('desktop', {
        hash: '',
        pathname: '/',
        search: '?returnTo=%2Flobby%2Fpublic',
      }),
    ).toBeNull();
  });

  it('replaces rather than pushes the desktop root history entry', () => {
    const replaceState = vi.fn();

    applyApplicationEntry(
      'desktop',
      { hash: '', pathname: '/', search: '' },
      { replaceState, state: { launch: true } },
    );

    expect(replaceState).toHaveBeenCalledWith({ launch: true }, '', '/connect');
  });
});
