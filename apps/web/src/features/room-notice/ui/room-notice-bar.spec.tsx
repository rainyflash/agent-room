// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { RoomNotice } from '@/features/room-notice/domain/room-notice';
import { RoomNoticeBar, type RoomNoticeBarProps } from '@/features/room-notice/ui/room-notice-bar';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

const ROOM_ID = '!club:matrix.test';
const SELF = '@mina:matrix.test';
const GRACE = '@grace:matrix.test';

const notice = (overrides: Partial<RoomNotice> = {}): RoomNotice => ({
  text: 'Only speak when someone mentions you.\nKeep replies short.\nAsk Grace first.',
  updatedAtUnixMs: Date.parse('2026-11-03T08:00:00.000Z'),
  updatedBy: GRACE,
  ...overrides,
});

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderBar(props: Partial<RoomNoticeBarProps> = {}) {
  return render(
    <I18nextProvider i18n={i18n}>
      <RoomNoticeBar
        memberNames={[{ displayName: 'Grace', matrixUserId: GRACE }]}
        notice={notice()}
        roomId={ROOM_ID}
        selfMatrixUserId={SELF}
        {...props}
      />
    </I18nextProvider>,
  );
}

describe('对话上方的公告', () => {
  it('平时只显示开头两行，没看过的亮点；点开看全文和谁改的，点灭，记在本机', () => {
    renderBar();
    const region = screen.getByRole('region', { name: 'Room notice' });
    const text = within(region).getByText(/Only speak when someone mentions you/u);
    expect(text).toHaveClass('room-notice__text--clamped');
    expect(within(region).getByText('Updated since you last looked')).toBeInTheDocument();
    expect(within(region).queryByText(/Updated by/u)).toBeNull();

    fireEvent.click(within(region).getByRole('button', { name: /Show the whole notice/u }));

    expect(text).not.toHaveClass('room-notice__text--clamped');
    expect(within(region).getByText(/^Updated by Grace · /u)).toBeVisible();
    expect(within(region).queryByText('Updated since you last looked')).toBeNull();
    expect(within(region).getByRole('button', { name: /Show less/u })).toHaveAttribute(
      'aria-expanded',
      'true',
    );

    cleanup();
    renderBar();
    expect(screen.queryByText('Updated since you last looked')).toBeNull();
  });

  it('改了新的一版又亮；自己改的不亮，写“你”', () => {
    window.localStorage.setItem(
      `agent-room.room-notice-seen.${ROOM_ID}`,
      String(Date.parse('2026-11-03T08:00:00.000Z')),
    );
    renderBar({ notice: notice({ updatedAtUnixMs: Date.parse('2026-11-03T09:00:00.000Z') }) });
    expect(screen.getByText('Updated since you last looked')).toBeInTheDocument();

    cleanup();
    renderBar({
      notice: notice({ updatedAtUnixMs: Date.parse('2026-11-03T10:00:00.000Z'), updatedBy: SELF }),
    });
    expect(screen.queryByText('Updated since you last looked')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Show the whole notice/u }));
    expect(screen.getByText(/^Updated by you · /u)).toBeVisible();
  });

  it('认不出改的人时写“房主或管理员”', () => {
    renderBar({ memberNames: undefined });
    fireEvent.click(screen.getByRole('button', { name: /Show the whole notice/u }));
    expect(screen.getByText(/^Updated by the owner or an admin · /u)).toBeVisible();
  });

  it('房主和管理员才有“编辑”', () => {
    renderBar();
    expect(screen.queryByRole('button', { name: 'Edit notice' })).toBeNull();

    cleanup();
    const onEdit = vi.fn();
    renderBar({ onEdit });
    fireEvent.click(screen.getByRole('button', { name: 'Edit notice' }));
    expect(onEdit).toHaveBeenCalledOnce();
  });
});
