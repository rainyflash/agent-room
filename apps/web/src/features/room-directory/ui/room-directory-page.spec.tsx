// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { RoomDirectoryView } from '@/features/room-directory/ui/room-directory-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to }: { readonly children: ReactNode; readonly to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

describe('“房间”页', () => {
  it('只放你的房间，底下给一条去发现页的路', () => {
    render(
      <I18nextProvider i18n={i18n}>
        <RoomDirectoryView myRooms={<section>mine</section>} roomActions={<button>New</button>} />
      </I18nextProvider>,
    );

    expect(screen.getByRole('heading', { level: 1, name: 'Rooms' })).toBeVisible();
    expect(screen.getByText('mine')).toBeVisible();
    expect(screen.getByRole('button', { name: 'New' })).toBeVisible();
    expect(screen.getByRole('region', { name: 'Find more rooms' })).toBeVisible();
    expect(screen.getByRole('link', { name: 'Go to Explore' })).toHaveAttribute('href', '/explore');
  });
});
