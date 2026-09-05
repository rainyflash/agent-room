// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import type { ComponentProps, ReactNode } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { RoomDirectoryView } from '@/features/room-directory/ui/room-directory-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

vi.mock('@tanstack/react-router', () => ({
  Link: ({ children, to }: { readonly children: ReactNode; readonly to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock('motion/react', () => ({
  motion: {
    article: ({ children, className }: ComponentProps<'article'>) => (
      <article className={className}>{children}</article>
    ),
  },
  useReducedMotion: () => true,
}));

const room: PublicRoomSummary = {
  activeInstanceCount: 1,
  catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
  description: 'A room backed by the live Matrix timeline.',
  language: 'en',
  name: 'Default public lobby',
  onlineAgentCount: 2,
  slug: 'default-public',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

describe('公共房间目录界面', () => {
  it('展示服务端房间事实和可操作入口', () => {
    renderView({ failureCode: null, loading: false, rooms: [room] });

    expect(screen.getByRole('heading', { name: 'Choose where your Agents meet.' })).toBeVisible();
    expect(screen.getByRole('heading', { name: 'Default public lobby' })).toBeVisible();
    expect(screen.getByText('2 Agents online')).toBeVisible();
    expect(screen.getByText('1 live instance')).toBeVisible();
    expect(screen.getByRole('link', { name: 'Enter room' })).toBeVisible();
  });

  it('目录故障时显示稳定诊断码和重试动作', () => {
    renderView({
      failureCode: 'room_directory.unreachable',
      loading: false,
      rooms: [],
    });

    expect(screen.getByText('The room directory could not be loaded')).toBeVisible();
    expect(screen.getByText('room_directory.unreachable')).toBeVisible();
    expect(screen.getAllByRole('button', { name: 'Refresh rooms' })).toHaveLength(2);
  });
});

function renderView(
  props: Pick<React.ComponentProps<typeof RoomDirectoryView>, 'failureCode' | 'loading' | 'rooms'>,
) {
  return render(
    <I18nextProvider i18n={i18n}>
      <RoomDirectoryView {...props} onRefresh={() => undefined} />
    </I18nextProvider>,
  );
}
