// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { BlockedAgent } from '@/features/direct-sessions/domain/direct-session';
import {
  BlockedAgentsPanel,
  type BlockedAgentsSource,
} from '@/features/security/ui/blocked-agents-panel';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const RECORDED: BlockedAgent = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e61',
  blockedAtUnixMs: Date.UTC(2026, 9, 24, 8, 0),
  displayName: 'Noisy helper #4821',
  matrixUserId: '@_agent_noisy:agent-room.test',
};
const IGNORED_ONLY: BlockedAgent = {
  agentId: null,
  blockedAtUnixMs: null,
  displayName: null,
  matrixUserId: '@_agent_unknown:agent-room.test',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('BlockedAgentsPanel', () => {
  it('一行一个：有名字写名字和屏蔽日期，不知道名字的写 Matrix ID；解除以后从列表里拿掉', async () => {
    const user = userEvent.setup();
    let blocks: readonly BlockedAgent[] = [RECORDED, IGNORED_ONLY];
    const unblock = vi.fn<BlockedAgentsSource['unblock']>((entry) => {
      blocks = blocks.filter((block) => block.matrixUserId !== entry.matrixUserId);
      return Promise.resolve(ok(undefined));
    });
    show({ listBlocked: () => Promise.resolve(ok(blocks)), unblock });

    const section = screen.getByRole('region', { name: 'Blocked agents' });
    const recorded = (await within(section).findByText('Noisy helper #4821')).closest('li');
    expect(recorded).toHaveTextContent('Blocked Oct 24, 2026');
    const ignoredOnly = within(section).getByText(IGNORED_ONLY.matrixUserId).closest('li');
    expect(ignoredOnly).not.toHaveTextContent('Blocked');

    if (recorded === null) throw new Error('应列出服务器记着的屏蔽');
    await user.click(within(recorded).getByRole('button', { name: 'Unblock' }));
    expect(unblock).toHaveBeenCalledWith(RECORDED);
    await waitFor(() => {
      expect(within(section).queryByText('Noisy helper #4821')).not.toBeInTheDocument();
    });
    expect(within(section).getByText(IGNORED_ONLY.matrixUserId)).toBeInTheDocument();
  });

  it('一个也没有时说怎么屏蔽；读不出来时给错误码和重试', async () => {
    show({ listBlocked: () => Promise.resolve(ok([])), unblock: vi.fn() });
    expect(
      await screen.findByText(
        'You haven’t blocked any agents. To block one, open it in a room and choose Block.',
      ),
    ).toBeInTheDocument();
    cleanup();

    const listBlocked = vi
      .fn<BlockedAgentsSource['listBlocked']>()
      .mockResolvedValueOnce(err({ code: 'direct_session.unreachable', retryable: true }))
      .mockResolvedValue(ok([RECORDED]));
    show({ listBlocked, unblock: vi.fn() });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Couldn’t load your blocked agents: direct_session.unreachable',
    );
    await userEvent.setup().click(screen.getByRole('button', { name: 'Retry' }));
    expect(await screen.findByText('Noisy helper #4821')).toBeInTheDocument();
  });

  it('解除没成时说没成，这一行留着', async () => {
    const user = userEvent.setup();
    show({
      listBlocked: () => Promise.resolve(ok([RECORDED])),
      unblock: () => Promise.resolve(err({ code: 'direct_session.unreachable', retryable: true })),
    });

    await user.click(await screen.findByRole('button', { name: 'Unblock' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Couldn’t unblock it: direct_session.unreachable',
    );
    expect(screen.getByText('Noisy helper #4821')).toBeInTheDocument();
  });
});

function show(source: BlockedAgentsSource) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <BlockedAgentsPanel source={source} />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}
