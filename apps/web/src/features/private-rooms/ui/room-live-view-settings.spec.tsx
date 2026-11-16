// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type {
  RoomLiveView,
  RoomLiveViewGateway,
} from '@/features/private-rooms/domain/room-live-view';
import { liveViewConditions } from '@/features/private-rooms/domain/room-live-view';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import {
  liveViewFailureMessage,
  RoomLiveViewFailureNotice,
  RoomLiveViewSettings,
} from './room-live-view-settings';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';

const off: RoomLiveView = {
  canChange: true,
  enabled: false,
  enabledAtUnixMs: null,
  live: false,
  unmetConditions: [],
};
const on: RoomLiveView = {
  ...off,
  enabled: true,
  enabledAtUnixMs: 1_790_000_000_000,
  live: true,
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
beforeEach(async () => {
  await i18n.changeLanguage('en');
});
afterEach(cleanup);

function gateway(initial: RoomLiveView) {
  let current = initial;
  const liveView = vi.fn<RoomLiveViewGateway['liveView']>(() => Promise.resolve(ok(current)));
  const updateLiveView = vi.fn<RoomLiveViewGateway['updateLiveView']>((_catalogId, enabled) => {
    current = enabled
      ? { ...current, enabled: true, enabledAtUnixMs: 1_790_000_100_000, live: true }
      : { ...current, enabled: false, enabledAtUnixMs: null, live: false };
    return Promise.resolve(ok(current));
  });
  return { liveView, updateLiveView, value: { liveView, updateLiveView } };
}

function renderGroup(live: RoomLiveViewGateway) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <RoomLiveViewSettings catalogId={CATALOG} gateway={live} />
      </I18nextProvider>
    </QueryClientProvider>,
  );
  return screen.getByRole('region', { name: 'Live view on the share page' });
}

const switchName = 'Show what’s said here on the share page';

describe('分享页放实况', () => {
  it('说明照设计写：不登录的人看得到以后说的话，加密、要申请、只能邀请的不能开', async () => {
    const group = renderGroup(gateway(off).value);
    expect(group).toHaveTextContent(
      'When it’s on, people who aren’t signed in can see what’s said here from then on, on the share page. Encrypted rooms and rooms people have to ask to join or be invited to can’t turn it on.',
    );
    expect(await within(group).findByRole('switch', { name: switchName })).not.toBeChecked();
    expect(within(group).getByRole('switch', { name: switchName })).toHaveAccessibleDescription(
      'Off.',
    );
  });

  it('打开先确认，说清楚不登录的人看得到什么；取消就不开', async () => {
    const live = gateway(off);
    const group = renderGroup(live.value);

    fireEvent.click(await within(group).findByRole('switch', { name: switchName }));
    const confirmation = within(group).getByRole('status');
    expect(confirmation).toHaveTextContent('Show what’s said here on the share page?');
    expect(confirmation).toHaveTextContent(
      'From now on, anyone can see what’s said in this room on its share page without signing in: the latest 20 messages, with the names of the people and agents who said them and when.',
    );
    expect(live.updateLiveView).not.toHaveBeenCalled();

    fireEvent.click(within(group).getByRole('button', { name: 'Cancel' }));
    expect(within(group).queryByRole('button', { name: 'Turn on' })).not.toBeInTheDocument();
    expect(within(group).getByRole('switch', { name: switchName })).not.toBeChecked();
    expect(live.updateLiveView).not.toHaveBeenCalled();

    fireEvent.click(within(group).getByRole('switch', { name: switchName }));
    fireEvent.click(within(group).getByRole('button', { name: 'Turn on' }));

    await waitFor(() => {
      expect(within(group).getByRole('switch', { name: switchName })).toHaveAccessibleDescription(
        /^On since .+\. Only what’s said after that is shown\.$/u,
      );
    });
    expect(live.updateLiveView).toHaveBeenCalledExactlyOnceWith(CATALOG, true);
    expect(within(group).getByRole('switch', { name: switchName })).toBeChecked();
    expect(within(group).queryByRole('button', { name: 'Turn on' })).not.toBeInTheDocument();
  });

  it('关掉不用确认', async () => {
    const live = gateway(on);
    const group = renderGroup(live.value);

    const toggle = await within(group).findByRole('switch', { name: switchName });
    expect(toggle).toBeChecked();
    fireEvent.click(toggle);

    await waitFor(() => {
      expect(within(group).getByRole('switch', { name: switchName })).not.toBeChecked();
    });
    expect(live.updateLiveView).toHaveBeenCalledExactlyOnceWith(CATALOG, false);
    expect(within(group).queryByRole('button', { name: 'Turn on' })).not.toBeInTheDocument();
  });

  it('关着又有不满足的：开关灰着，一样一样照实说', async () => {
    const group = renderGroup(
      gateway({ ...off, unmetConditions: ['encrypted', 'join_mode'] }).value,
    );

    expect(await within(group).findByRole('switch', { name: switchName })).toBeDisabled();
    const reasons = within(group).getAllByRole('listitem');
    expect(reasons.map((item) => item.textContent)).toEqual([
      'This room is end-to-end encrypted, and encrypted rooms can’t show a live view.',
      'Only rooms people can join directly can show a live view. Set How people join to Join directly first.',
    ]);
  });

  it('开着却被平台拿掉：说现在没在放、为什么，照样能关', async () => {
    const group = renderGroup(
      gateway({ ...on, live: false, unmetConditions: ['platform_hold'] }).value,
    );

    const toggle = await within(group).findByRole('switch', { name: switchName });
    expect(toggle).toBeChecked();
    expect(toggle).toBeEnabled();
    expect(toggle).toHaveAccessibleDescription('On, but not shown on the share page right now.');
    expect(within(group).getByRole('listitem')).toHaveTextContent(
      'Agent Room took this room off Explore, so the live view is paused.',
    );
  });

  it('不是房主或管理员：只看得到开没开，没有开关', async () => {
    const group = renderGroup(gateway({ ...on, canChange: false }).value);

    expect(
      await within(group).findByText(/^On since .+\. Only what’s said after that is shown\.$/u),
    ).toBeInTheDocument();
    expect(within(group).queryByRole('switch')).not.toBeInTheDocument();
    expect(group).toHaveTextContent('Only the owner and admins can turn it on or off.');
  });

  it('开不了（409）：说差的是哪一样，再问一次服务器照新的样子画', async () => {
    const live = gateway(off);
    live.updateLiveView.mockResolvedValueOnce(
      err({ code: 'room_live.not_eligible', condition: 'join_mode', retryable: false }),
    );
    const group = renderGroup(live.value);

    fireEvent.click(await within(group).findByRole('switch', { name: switchName }));
    // 服务器那边刚换成了要申请：下一次读就带着这一样。
    live.liveView.mockResolvedValue(ok({ ...off, unmetConditions: ['join_mode'] }));
    fireEvent.click(within(group).getByRole('button', { name: 'Turn on' }));

    expect(
      await within(group).findByText(
        'Only rooms people can join directly can show a live view. Set How people join to Join directly first.',
        { selector: '.ar-banner p' },
      ),
    ).toBeInTheDocument();
    expect(within(group).getByText('room_live.not_eligible')).toBeInTheDocument();
    await waitFor(() => {
      expect(within(group).getByRole('switch', { name: switchName })).toBeDisabled();
    });
    expect(live.liveView).toHaveBeenCalledTimes(2);
  });

  it('读不出来时说清楚，能重试', async () => {
    const live = gateway(off);
    live.liveView.mockResolvedValueOnce(
      err({ code: 'room_live.dependency_unavailable', retryable: true }),
    );
    const group = renderGroup(live.value);

    expect(
      await within(group).findByText(
        'The live view couldn’t be changed right now. Try again in a moment.',
      ),
    ).toBeInTheDocument();
    fireEvent.click(within(group).getByRole('button', { name: 'Try again' }));
    expect(await within(group).findByRole('switch', { name: switchName })).not.toBeChecked();
  });
});

describe('实况的错误说法', () => {
  const codes = [
    'room_live.conflict',
    'room_live.dependency_unavailable',
    'room_live.forbidden',
    'room_live.internal',
    'room_live.invalid',
    'room_live.invalid_origin',
    'room_live.invalid_resource',
    'room_live.not_eligible',
    'room_live.not_found',
  ];

  it('每个 room_live 错误码中英文都有一句大白话', async () => {
    for (const language of ['en', 'zh-CN'] as const) {
      await initializeI18n(window.localStorage, [language]);
      await i18n.changeLanguage(language);
      for (const code of codes) {
        const key = liveViewFailureMessage({ code, retryable: false });
        expect(key, code).not.toBeNull();
        expect(i18n.exists(key ?? '', { lng: language, fallbackLng: false }), code).toBe(true);
      }
      for (const condition of liveViewConditions) {
        const key = liveViewFailureMessage({
          code: 'room_live.not_eligible',
          condition,
          retryable: false,
        });
        expect(key).toBe(`roomLive.condition.${condition}`);
        expect(i18n.exists(key ?? '', { lng: language, fallbackLng: false }), condition).toBe(true);
      }
    }
  });

  it('不认得的条件照通用的“开不了”说；别的错误码交给私人房间的通用说法', async () => {
    await i18n.changeLanguage('zh-CN');
    const failures: readonly PrivateRoomFailure[] = [
      { code: 'room_live.not_eligible', condition: 'moon_phase', retryable: false },
      { code: 'room_live.not_eligible', condition: 'encrypted', retryable: false },
      { code: 'private_room.unreachable', retryable: true },
    ];
    const texts = failures.map((failure) => {
      const view = render(
        <I18nextProvider i18n={i18n}>
          <RoomLiveViewFailureNotice failure={failure} />
        </I18nextProvider>,
      );
      const text = view.container.textContent;
      cleanup();
      return text;
    });
    expect(texts[0]).toContain('这间房现在不能放实况。');
    expect(texts[1]).toContain('这间房是端到端加密的，加密的房间不能放实况。');
    expect(texts[2]).toContain('private_room.unreachable');
    expect(
      liveViewFailureMessage({ code: 'private_room.unreachable', retryable: true }),
    ).toBeNull();
  });
});
