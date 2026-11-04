// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { RoomNotice, RoomNoticeGateway } from '@/features/room-notice/domain/room-notice';
import { RoomNoticeEditor } from '@/features/room-notice/ui/room-notice-editor';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const CURRENT: RoomNotice = {
  text: 'Only speak when someone mentions you.',
  updatedAtUnixMs: 1_000,
  updatedBy: '@grace:matrix.test',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function wrap(element: ReactElement) {
  return (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>{element}</QueryClientProvider>
    </I18nextProvider>
  );
}

function writes(text: string, updatedAtUnixMs = 2_000): RoomNoticeGateway['updateNotice'] {
  return vi.fn(() => Promise.resolve(ok({ text, updatedAtUnixMs })));
}

describe('房间设置里的公告', () => {
  it('没改不能保存；改了发去掉首尾空白、回车换成换行的正文，同步回来之前照写进去的显示', async () => {
    const updateNotice = writes('Reply in English.\nKeep it short.');
    const view = render(
      wrap(<RoomNoticeEditor catalogId={CATALOG} gateway={{ updateNotice }} notice={CURRENT} />),
    );
    const save = screen.getByRole('button', { name: 'Save notice' });
    expect(save).toBeDisabled();
    expect(screen.getByText(/Notices aren’t encrypted/u)).toBeVisible();

    fireEvent.change(screen.getByRole('textbox', { name: 'Notice' }), {
      target: { value: '  Reply in English.\r\nKeep it short.  ' },
    });
    fireEvent.click(save);

    expect(await screen.findByText('Notice updated.')).toBeVisible();
    expect(updateNotice).toHaveBeenCalledWith(CATALOG, 'Reply in English.\nKeep it short.');
    expect(screen.getByRole('textbox', { name: 'Notice' })).toHaveValue(
      'Reply in English.\nKeep it short.',
    );
    expect(screen.getByRole('button', { name: 'Save notice' })).toBeDisabled();

    // 同步回来的新一版接着当底稿。
    view.rerender(
      wrap(
        <RoomNoticeEditor
          catalogId={CATALOG}
          gateway={{ updateNotice }}
          notice={{ ...CURRENT, text: 'Reply in English.\nKeep it short.', updatedAtUnixMs: 2_000 }}
        />,
      ),
    );
    expect(screen.getByRole('textbox', { name: 'Notice' })).toHaveValue(
      'Reply in English.\nKeep it short.',
    );
  });

  it('撤掉公告发空的正文；没有公告时没有这个按钮', async () => {
    const updateNotice = writes('');
    render(
      wrap(<RoomNoticeEditor catalogId={CATALOG} gateway={{ updateNotice }} notice={CURRENT} />),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove notice' }));

    expect(await screen.findByText('Notice removed.')).toBeVisible();
    expect(updateNotice).toHaveBeenCalledWith(CATALOG, '');
    expect(screen.getByRole('textbox', { name: 'Notice' })).toHaveValue('');
    expect(screen.queryByRole('button', { name: 'Remove notice' })).toBeNull();
  });

  it('太长的、带特殊字符的先说清楚，不发出去', () => {
    const updateNotice = writes('');
    render(
      wrap(<RoomNoticeEditor catalogId={CATALOG} gateway={{ updateNotice }} notice={undefined} />),
    );
    const field = screen.getByRole('textbox', { name: 'Notice' });

    fireEvent.change(field, { target: { value: '公'.repeat(2_001) } });
    expect(screen.getByRole('alert')).toHaveTextContent(
      'A notice can be at most 2,000 characters.',
    );
    expect(screen.getByText('2,001 / 2,000')).toBeVisible();
    expect(screen.getByRole('button', { name: 'Save notice' })).toBeDisabled();

    fireEvent.change(field, { target: { value: 'Bell\u0007' } });
    expect(screen.getByRole('alert')).toHaveTextContent('Remove the special characters.');
    expect(screen.getByRole('button', { name: 'Save notice' })).toBeDisabled();
    expect(updateNotice).not.toHaveBeenCalled();
  });

  it.each([
    ['room_notice.rate_limited', 'You’ve changed the notice many times in the past hour.'],
    ['room_notice.dependency_unavailable', 'The notice wasn’t changed. Try again.'],
    ['private_room.unreachable', 'The notice wasn’t changed. Try again.'],
    ['room_notice.conflict', 'This room is archived, so its notice can’t be changed.'],
    ['room_notice.forbidden', 'Only the owner and admins can change the notice.'],
    ['room_notice.internal', 'The notice wasn’t changed.'],
  ])('没改成时照错误码说：%s', async (code, message) => {
    const updateNotice = vi.fn(() => Promise.resolve(err({ code, retryable: true })));
    render(
      wrap(<RoomNoticeEditor catalogId={CATALOG} gateway={{ updateNotice }} notice={CURRENT} />),
    );
    fireEvent.change(screen.getByRole('textbox', { name: 'Notice' }), {
      target: { value: 'New rules' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save notice' }));

    expect(await screen.findByText(new RegExp(escape(message), 'u'))).toBeVisible();
    // 草稿还在，可以再点。
    expect(screen.getByRole('textbox', { name: 'Notice' })).toHaveValue('New rules');
    expect(screen.getByRole('button', { name: 'Save notice' })).toBeEnabled();
  });
});

function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
}
