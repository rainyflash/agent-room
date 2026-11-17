// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { roomRepositoryQueryKey } from '@/features/private-rooms/data/project-room-queries';
import type {
  ProjectRoomGateway,
  RoomRepositoryBinding,
} from '@/features/private-rooms/domain/project-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { RepositoryBadgeShare } from './project-room-badge';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';

const BADGE =
  '[![Agent Room](https://agentroom.test/gh/rainyflash/agent-room/badge.svg)](https://agentroom.test/r/aB3xYz)';

const bound: RoomRepositoryBinding = {
  agentsLine:
    "Agent Room: this project's room is https://agentroom.test/r/aB3xYz. Agents join with `agent-room join https://agentroom.test/r/aB3xYz`, or over the network via https://agentroom.test/agents.txt",
  badgeMarkdown: BADGE,
  repository: 'rainyflash/agent-room',
  repositoryUrl: 'https://github.com/rainyflash/agent-room',
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
  verifiedAtUnixMs: 1_790_000_600_000,
  verifiedByAccountId: OWNER,
  verifiedFile: 'AGENTS.md',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});
afterEach(() => {
  cleanup();
  Reflect.deleteProperty(navigator, 'clipboard');
});

function gateway(binding: ProjectRoomGateway['binding']): ProjectRoomGateway {
  const unused = vi.fn(() => Promise.reject(new Error('not used here')));
  return {
    binding,
    cancelClaim: unused,
    checkBinding: unused,
    checkClaim: unused,
    claim: unused,
    claims: unused,
    inspectClaim: unused,
    roomRepository: unused,
    unbind: unused,
  };
}

function renderSection(projects: ProjectRoomGateway, client = new QueryClient()) {
  return render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <div data-testid="share-dialog">
          <RepositoryBadgeShare catalogId={CATALOG} gateway={projects} />
        </div>
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

describe('分享对话框里的“README 徽章”', () => {
  it('绑着仓库：标题、一句说明、服务器给的那一行和“复制徽章”，复制的是原文', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    const binding = vi.fn<ProjectRoomGateway['binding']>(() => Promise.resolve(ok(bound)));
    renderSection(gateway(binding));

    const section = await screen.findByRole('region', { name: 'README badge' });
    expect(within(section).getByRole('heading', { name: 'README badge' })).toBeVisible();
    expect(section).toHaveTextContent(
      'Put it in the repository’s README. The badge shows how many are online here, and clicking it opens the link above.',
    );
    expect(within(section).getByLabelText('README badge')).toHaveTextContent(BADGE);
    expect(binding).toHaveBeenCalledWith(CATALOG);

    fireEvent.click(within(section).getByRole('button', { name: 'Copy badge' }));
    expect(await within(section).findByRole('button', { name: 'Copied' })).toBeVisible();
    expect(writeText).toHaveBeenCalledWith(BADGE);
  });

  it('设置里那一组问过了：用缓存里的那一份，不再问一次', async () => {
    const binding = vi.fn<ProjectRoomGateway['binding']>(() => Promise.resolve(ok(bound)));
    const client = new QueryClient();
    client.setQueryData(roomRepositoryQueryKey(CATALOG), ok(bound));
    renderSection(gateway(binding), client);

    expect(await screen.findByRole('region', { name: 'README badge' })).toBeVisible();
    expect(binding).not.toHaveBeenCalled();
  });

  it('没绑、没有那一行、问不到：整节不出来', async () => {
    const cases: readonly ProjectRoomGateway['binding'][] = [
      () => Promise.resolve(ok(null)),
      () => Promise.resolve(ok({ ...bound, badgeMarkdown: null })),
      () => Promise.resolve(err({ code: 'private_room.forbidden', retryable: false })),
      () => Promise.reject(new Error('offline')),
    ];
    for (const answer of cases) {
      const binding = vi.fn(answer);
      const view = renderSection(gateway(binding));
      await waitFor(() => {
        expect(binding).toHaveBeenCalled();
      });
      // 等这一次问完：结果出来以后也什么都没画。
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(view.getByTestId('share-dialog')).toBeEmptyDOMElement();
      view.unmount();
    }
  });

  it('中文：标题、说明和按钮', async () => {
    await i18n.changeLanguage('zh-CN');
    renderSection(gateway(() => Promise.resolve(ok(bound))));

    const section = await screen.findByRole('region', { name: 'README 徽章' });
    expect(section).toHaveTextContent(
      '放进仓库的 README：徽章上写着这里现在几个在线，点它打开的就是上面的链接。',
    );
    expect(within(section).getByRole('button', { name: '复制徽章' })).toBeVisible();
  });
});
