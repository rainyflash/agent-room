// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type {
  RoomExcerpt,
  RoomExcerptGateway,
} from '@/features/room-excerpts/domain/room-excerpts';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { useExcerptFailureMessage } from './excerpt-failure';
import { SharedExcerpts } from './shared-excerpts';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const AT = new Date(2026, 10, 16, 9, 30).getTime();

const MINE: RoomExcerpt = {
  code: 'Kq7Lm2Zp',
  url: 'https://agentroom.test/r/aB3xYz/s/Kq7Lm2Zp',
  messageCount: 4,
  firstMessageAtUnixMs: AT,
  sharedAtUnixMs: AT + 60_000,
  mine: true,
};
const THEIRS: RoomExcerpt = {
  ...MINE,
  code: 'Wd4Rt8Nx',
  url: 'https://agentroom.test/r/aB3xYz/s/Wd4Rt8Nx',
  messageCount: 1,
  mine: false,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function gateway(
  initial: readonly RoomExcerpt[],
  remove: RoomExcerptGateway['remove'] = vi.fn(() => Promise.resolve(ok(undefined))),
) {
  let current = initial;
  const list = vi.fn<RoomExcerptGateway['list']>(() => Promise.resolve(ok(current)));
  const removing = vi.fn<RoomExcerptGateway['remove']>(async (catalogId, code) => {
    const result = await remove(catalogId, code);
    if (result.ok) current = current.filter((excerpt) => excerpt.code !== code);
    return result;
  });
  return { list, remove: removing, share: vi.fn() } satisfies RoomExcerptGateway;
}

function renderSection(source: RoomExcerptGateway, props: { manages: boolean; live: boolean }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <SharedExcerpts
          catalogId={CATALOG}
          gateway={source}
          live={props.live}
          manages={props.manages}
        />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('分享过的几段', () => {
  it('成员只看得到自己的：标题是“你分享过的”，不标“你的”', async () => {
    renderSection(gateway([MINE]), { manages: false, live: true });
    const section = await screen.findByRole('region', { name: 'Stretches you’ve shared' });
    expect(section).toHaveTextContent('Only you see this list.');
    const row = await within(section).findByRole('group', { name: /^4 messages · / });
    expect(row).not.toHaveTextContent('Yours');
    expect(
      within(row).getByRole('link', { name: 'agentroom.test/r/aB3xYz/s/Kq7Lm2Zp' }),
    ).toHaveAttribute('href', MINE.url);
    expect(within(row).getByRole('button', { name: 'Copy link' })).toBeVisible();
  });

  it('房主和管理员看得到这间房的全部：标题换成“这间房分享过的”，自己的标出来', async () => {
    renderSection(gateway([MINE, THEIRS]), { manages: true, live: true });
    const section = await screen.findByRole('region', { name: 'Shared from this room' });
    expect(section).toHaveTextContent('but not who shared it');
    const rows = await within(section).findAllByRole('group');
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('Yours');
    expect(rows[1]).toHaveTextContent(/^1 message · /);
    expect(rows[1]).not.toHaveTextContent('Yours');
  });

  it('房间没有分享码时这一段没有链接，也不给复制', async () => {
    renderSection(gateway([{ ...MINE, url: null }]), { manages: false, live: true });
    const row = await screen.findByRole('group', { name: /^4 messages · / });
    expect(row).toHaveTextContent('No link right now: this room has no share link.');
    expect(within(row).queryByRole('link')).toBeNull();
    expect(within(row).queryByRole('button', { name: 'Copy link' })).toBeNull();
  });

  it('删掉先在这一行里确认：焦点落在“不删了”，不删回到“删掉”；删了从名单上拿掉，读屏说一声', async () => {
    const source = gateway([MINE, THEIRS]);
    renderSection(source, { manages: true, live: true });
    const first = await screen.findByRole('group', { name: /^4 messages · / });

    fireEvent.click(within(first).getByRole('button', { name: 'Delete' }));
    expect(first).toHaveTextContent('Delete it? The link stops working right away.');
    await waitFor(() => {
      expect(within(first).getByRole('button', { name: 'Keep it' })).toHaveFocus();
    });
    fireEvent.click(within(first).getByRole('button', { name: 'Keep it' }));
    await waitFor(() => {
      expect(within(first).getByRole('button', { name: 'Delete' })).toHaveFocus();
    });
    expect(source.remove).not.toHaveBeenCalled();

    fireEvent.click(within(first).getByRole('button', { name: 'Delete' }));
    fireEvent.click(within(first).getByRole('button', { name: 'Delete' }));
    await waitFor(() => {
      expect(screen.getAllByRole('group')).toHaveLength(1);
    });
    expect(source.remove).toHaveBeenCalledWith(CATALOG, MINE.code);
    const section = screen.getByRole('region', { name: 'Shared from this room' });
    expect(within(section).getByRole('status')).toHaveTextContent(
      'Deleted. The link no longer opens.',
    );
    expect(within(section).getByRole('heading', { name: 'Shared from this room' })).toHaveFocus();
  });

  it('删不掉时在这一行里说为什么；早就没了的直接从名单上拿掉', async () => {
    const forbidden = gateway([MINE], () =>
      Promise.resolve(
        err({ code: 'room_excerpt.forbidden', correlationId: 'corr-2', retryable: false }),
      ),
    );
    renderSection(forbidden, { manages: false, live: true });
    const row = await screen.findByRole('group', { name: /^4 messages · / });
    fireEvent.click(within(row).getByRole('button', { name: 'Delete' }));
    fireEvent.click(within(row).getByRole('button', { name: 'Delete' }));
    const alert = await within(row).findByRole('alert');
    expect(alert).toHaveTextContent('Couldn’t delete it');
    expect(alert).toHaveTextContent(
      'Only the person who shared it, the owner and admins can delete it.',
    );
    cleanup();

    let gone = false;
    const missing = {
      list: vi.fn<RoomExcerptGateway['list']>(() => Promise.resolve(ok(gone ? [] : [MINE]))),
      remove: vi.fn<RoomExcerptGateway['remove']>(() => {
        gone = true;
        return Promise.resolve(err({ code: 'room_excerpt.not_found', retryable: false }));
      }),
      share: vi.fn(),
    } satisfies RoomExcerptGateway;
    renderSection(missing, { manages: false, live: true });
    const stale = await screen.findByRole('group', { name: /^4 messages · / });
    fireEvent.click(within(stale).getByRole('button', { name: 'Delete' }));
    fireEvent.click(within(stale).getByRole('button', { name: 'Delete' }));
    await waitFor(() => {
      expect(screen.queryByRole('group')).toBeNull();
    });
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('一段也没有：放着实况时说怎么分享；没在放时整节不出来', async () => {
    renderSection(gateway([]), { manages: false, live: true });
    expect(
      await screen.findByText(
        'Nothing yet. Point at a message in the conversation and choose Share this stretch.',
      ),
    ).toBeVisible();
    cleanup();

    const source = gateway([]);
    const { container } = renderSection(source, { manages: true, live: false });
    await waitFor(() => {
      expect(source.list).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(container).toBeEmptyDOMElement();
    });
  });

  it('读不出来时给“再试一次”', async () => {
    const source = gateway([]);
    source.list.mockResolvedValueOnce(err({ code: 'private_room.unreachable', retryable: true }));
    renderSection(source, { manages: false, live: true });
    expect(await screen.findByText('Couldn’t load what’s been shared.')).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(
      await screen.findByText(
        'Nothing yet. Point at a message in the conversation and choose Share this stretch.',
      ),
    ).toBeVisible();
  });
});

describe('每个错误码都换成大白话，两种语言', () => {
  const failures: readonly PrivateRoomFailure[] = [
    { code: 'room_excerpt.not_live', retryable: false },
    { code: 'room_excerpt.too_long', retryable: false },
    { code: 'room_excerpt.invalid_range', reason: 'before_live', retryable: false },
    { code: 'room_excerpt.invalid_range', reason: 'not_found', retryable: false },
    { code: 'room_excerpt.rate_limited', retryAfterSeconds: 60, retryable: true },
    { code: 'room_excerpt.rate_limited', retryAfterSeconds: 1200, retryable: true },
    { code: 'room_excerpt.rate_limited', retryable: true },
    { code: 'room_excerpt.not_found', retryable: false },
    { code: 'room_excerpt.forbidden', retryable: false },
    { code: 'room_excerpt.dependency_unavailable', retryable: true },
    { code: 'room_excerpt.internal', retryable: true },
    { code: 'room_excerpt.invalid', retryable: false },
    { code: 'room_excerpt.invalid_resource', retryable: false },
    { code: 'room_excerpt.invalid_origin', retryable: false },
    { code: 'private_room.invalid_response', retryable: false },
    { code: 'private_room.unreachable', retryable: true },
    { code: 'private_room.http_418', retryable: false },
  ];

  function messages(action: 'share' | 'delete') {
    const wrapper = ({ children }: { readonly children: ReactNode }) => (
      <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
    );
    const { result } = renderHook(() => useExcerptFailureMessage(), { wrapper });
    return failures.map((failure) => result.current(failure, action));
  }

  it('英文：开实况以前的话只说只能分享开了以后说的；太多了说几分钟以后', () => {
    const shared = messages('share');
    expect(shared[2]).toBe(
      'Only messages said after the live view was turned on can be shared. Start from a later message.',
    );
    expect(shared[4]).toMatch(/Try again in 1 minute\.$/u);
    expect(shared[5]).toMatch(/Try again in 20 minutes\.$/u);
    expect(shared[6]).toMatch(/Try again later\.$/u);
    for (const text of shared) {
      expect(text).not.toMatch(/room_excerpt|private_room|roomExcerpt\./u);
      expect(text.length).toBeGreaterThan(0);
    }
    const deleted = messages('delete');
    expect(deleted[7]).toBe('It’s already gone.');
    expect(deleted[8]).toBe('Only the person who shared it, the owner and admins can delete it.');
  });

  it('中文：每一句都有，不露出键名和错误码', async () => {
    await i18n.changeLanguage('zh-CN');
    const shared = messages('share');
    expect(shared[2]).toContain('只能分享开了实况以后说的话');
    expect(shared[5]).toContain('20');
    for (const text of [...shared, ...messages('delete')]) {
      expect(text).not.toMatch(/room_excerpt|private_room|roomExcerpt\.|[A-Za-z]{6,}/u);
    }
  });
});
