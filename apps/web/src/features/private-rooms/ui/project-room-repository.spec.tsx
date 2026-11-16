// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { rememberClaim, rememberedClaim } from '@/features/private-rooms/data/project-room-queries';
import type {
  ProjectRoomGateway,
  RepositoryClaim,
  RoomRepositoryBinding,
} from '@/features/private-rooms/domain/project-rooms';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { ProjectRoomRepository } from './project-room-repository';

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const FORMER_OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const CLAIM = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';

const BADGE =
  '[![Agent Room](https://agentroom.test/gh/rainyflash/agent-room/badge.svg)](https://agentroom.test/r/aB3xYz)';
const AGENTS_LINE =
  "Agent Room: this project's room is https://agentroom.test/r/aB3xYz. Agents join with `agent-room join https://agentroom.test/r/aB3xYz`, or over the network via https://agentroom.test/agents.txt";

const pending: RepositoryClaim = {
  agentsLine: AGENTS_LINE,
  badgeMarkdown: BADGE,
  catalogId: CATALOG,
  claimId: CLAIM,
  claimedAtUnixMs: 1_790_000_000_000,
  lastCheck: null,
  repository: 'rainyflash/agent-room',
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
  status: 'pending',
};

const bound: RoomRepositoryBinding = {
  agentsLine: AGENTS_LINE,
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
beforeEach(() => {
  window.localStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function gateway(overrides: Partial<ProjectRoomGateway> = {}) {
  const value: ProjectRoomGateway = {
    binding: vi.fn(() => Promise.resolve(ok(null))),
    cancelClaim: vi.fn(() => Promise.resolve(ok(undefined))),
    checkBinding: vi.fn(() =>
      Promise.resolve(
        ok({ binding: bound, lastCheck: { atUnixMs: Date.now(), result: 'found' as const } }),
      ),
    ),
    checkClaim: vi.fn(() => Promise.resolve(ok(pending))),
    claim: vi.fn(() => Promise.resolve(ok(pending))),
    inspectClaim: vi.fn(() => Promise.resolve(ok(pending))),
    unbind: vi.fn(() => Promise.resolve(ok(undefined))),
    ...overrides,
  };
  return value;
}

function renderGroup(projects: ProjectRoomGateway) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <ProjectRoomRepository catalogId={CATALOG} gateway={projects} principalId={OWNER} />
      </I18nextProvider>
    </QueryClientProvider>,
  );
}

function lineOf(label: string): HTMLElement {
  const block = screen.getByLabelText(label).closest<HTMLElement>('.room-repository__line');
  if (block === null) throw new Error(`no line for ${label}`);
  return block;
}

describe('ProjectRoomRepository', () => {
  it('还没绑：填 owner/repo 点“绑上”，给出徽章和 AGENTS.md 那一行，记下在等的认领', async () => {
    const claim = vi.fn<ProjectRoomGateway['claim']>(() => Promise.resolve(ok(pending)));
    renderGroup(gateway({ claim }));

    expect(await screen.findByRole('heading', { name: 'GitHub repository' })).toBeVisible();
    fireEvent.change(await screen.findByRole('textbox', { name: 'Repository' }), {
      target: { value: ' https://github.com/rainyflash/agent-room.git ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Link' }));

    expect(await screen.findByText('Waiting for rainyflash/agent-room')).toBeVisible();
    expect(claim).toHaveBeenCalledWith({
      catalogId: CATALOG,
      repository: 'rainyflash/agent-room',
    });
    expect(screen.getByLabelText('README badge')).toHaveTextContent(BADGE);
    expect(screen.getByLabelText('Line for AGENTS.md')).toHaveTextContent(AGENTS_LINE);
    expect(
      screen.getByText('It checks by itself after you push, or you can check now.'),
    ).toBeVisible();
    expect(screen.getByText('Not checked yet.')).toBeVisible();
    expect(rememberedClaim(OWNER, CATALOG)).toBe(CLAIM);
  });

  it('填得不像 owner/repo：就地说怎么写，不发请求', async () => {
    const claim = vi.fn<ProjectRoomGateway['claim']>(() => Promise.resolve(ok(pending)));
    renderGroup(gateway({ claim }));

    const input = await screen.findByRole('textbox', { name: 'Repository' });
    fireEvent.change(input, { target: { value: 'agent-room' } });
    fireEvent.click(screen.getByRole('button', { name: 'Link' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Write it as owner/repo, for example rainyflash/agent-room.',
    );
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(claim).not.toHaveBeenCalled();
  });

  it('两行各有复制按钮，复制的是原文', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    rememberClaim(OWNER, CATALOG, CLAIM);
    renderGroup(gateway());

    await screen.findByText('Waiting for rainyflash/agent-room');
    fireEvent.click(within(lineOf('README badge')).getByRole('button', { name: 'Copy' }));
    expect(
      await within(lineOf('README badge')).findByRole('button', { name: 'Copied' }),
    ).toBeVisible();
    fireEvent.click(within(lineOf('Line for AGENTS.md')).getByRole('button', { name: 'Copy' }));
    await within(lineOf('Line for AGENTS.md')).findByRole('button', { name: 'Copied' });

    expect(writeText).toHaveBeenNthCalledWith(1, BADGE);
    expect(writeText).toHaveBeenNthCalledWith(2, AGENTS_LINE);
  });

  it('在等：写着上次核对的结果；“现在核对”太快时按 Retry-After 灰着按钮、数秒', async () => {
    rememberClaim(OWNER, CATALOG, CLAIM);
    const projects = gateway({
      checkClaim: vi.fn(() =>
        Promise.resolve(
          err({ code: 'project_room.check_too_soon', retryable: true, retryAfterSeconds: 3 }),
        ),
      ),
      inspectClaim: vi.fn(() =>
        Promise.resolve(
          ok({
            ...pending,
            lastCheck: { atUnixMs: 1_790_000_300_000, result: 'not_in_repository' as const },
          }),
        ),
      ),
    });
    renderGroup(projects);

    expect(await screen.findByText(/the line isn’t in the repository yet/u)).toBeVisible();
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] });
    fireEvent.click(screen.getByRole('button', { name: 'Check now' }));

    expect(
      await screen.findByText('You can check once a minute. Try again in 3 seconds.'),
    ).toBeVisible();
    expect(screen.getByRole('button', { name: 'Check now' })).toBeDisabled();
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(screen.getByText('You can check once a minute. Try again in 2 seconds.')).toBeVisible();
    act(() => {
      vi.advanceTimersByTime(2_000);
    });
    expect(screen.queryByText(/Try again in/u)).toBeNull();
    expect(screen.getByRole('button', { name: 'Check now' })).toBeEnabled();
    // 太快不是出错：不出红色的失败提示。
    expect(screen.queryByText('project_room.check_too_soon')).toBeNull();
  });

  it('“现在核对”认上了：忘掉认领，换成绑上了的样子', async () => {
    rememberClaim(OWNER, CATALOG, CLAIM);
    let binding: RoomRepositoryBinding | null = null;
    const projects = gateway({
      binding: vi.fn(() => Promise.resolve(ok(binding))),
      checkClaim: vi.fn(() => {
        binding = bound;
        return Promise.resolve(
          ok({ ...pending, claimId: null, claimedAtUnixMs: null, status: 'bound' as const }),
        );
      }),
    });
    renderGroup(projects);

    fireEvent.click(await screen.findByRole('button', { name: 'Check now' }));

    expect(
      await screen.findByRole('link', {
        name: 'Open rainyflash/agent-room on GitHub (opens in a new tab)',
      }),
    ).toBeVisible();
    expect(rememberedClaim(OWNER, CATALOG)).toBeNull();
  });

  it('“不绑了”：撤回认领，回到输入框', async () => {
    rememberClaim(OWNER, CATALOG, CLAIM);
    const cancelClaim = vi.fn<ProjectRoomGateway['cancelClaim']>(() =>
      Promise.resolve(ok(undefined)),
    );
    renderGroup(gateway({ cancelClaim }));

    fireEvent.click(await screen.findByRole('button', { name: 'Stop linking' }));

    expect(await screen.findByRole('textbox', { name: 'Repository' })).toBeVisible();
    expect(cancelClaim).toHaveBeenCalledWith(CLAIM);
    expect(rememberedClaim(OWNER, CATALOG)).toBeNull();
  });

  it('记着的认领在服务器上没了：回到输入框，说一句为什么', async () => {
    rememberClaim(OWNER, CATALOG, CLAIM);
    renderGroup(
      gateway({
        inspectClaim: vi.fn(() =>
          Promise.resolve(err({ code: 'project_room.not_found', retryable: false })),
        ),
      }),
    );

    expect(await screen.findByText(/This link request is gone/u)).toBeVisible();
    expect(screen.getByRole('textbox', { name: 'Repository' })).toBeVisible();
    expect(rememberedClaim(OWNER, CATALOG)).toBeNull();
  });

  it('认领被拒：按错误码说人话，错误码收进详情', async () => {
    renderGroup(
      gateway({
        claim: vi.fn(() =>
          Promise.resolve(err({ code: 'project_room.too_many_claims', retryable: false })),
        ),
      }),
    );

    fireEvent.change(await screen.findByRole('textbox', { name: 'Repository' }), {
      target: { value: 'rainyflash/agent-room' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Link' }));

    expect(await screen.findByText(/already have 3 repositories waiting/u)).toBeVisible();
    expect(screen.getByText('project_room.too_many_claims')).not.toBeVisible();
  });

  it('绑上了：仓库、谁什么时候在哪个文件里核对上；“再核对一次”说结果', async () => {
    const projects = gateway({ binding: vi.fn(() => Promise.resolve(ok(bound))) });
    renderGroup(projects);

    const link = await screen.findByRole('link', {
      name: 'Open rainyflash/agent-room on GitHub (opens in a new tab)',
    });
    expect(link).toHaveAttribute('href', 'https://github.com/rainyflash/agent-room');
    expect(link).toHaveAttribute('target', '_blank');
    expect(screen.getByText(/^Verified by you on .+, found in AGENTS\.md\.$/u)).toBeVisible();
    expect(screen.queryByRole('textbox')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Check again' }));
    expect(await screen.findByText(/the link is still in AGENTS\.md\./u)).toBeVisible();
  });

  it('换了房主以后：写“另一个账户”，账户 ID 收进详情', async () => {
    renderGroup(
      gateway({
        binding: vi.fn(() => Promise.resolve(ok({ ...bound, verifiedByAccountId: FORMER_OWNER }))),
      }),
    );

    expect(await screen.findByText(/^Verified by another account on/u)).toBeVisible();
    expect(screen.getByText(FORMER_OWNER)).not.toBeVisible();
  });

  it('再核对没找到：照实说，绑定照旧', async () => {
    renderGroup(
      gateway({
        binding: vi.fn(() => Promise.resolve(ok(bound))),
        checkBinding: vi.fn(() =>
          Promise.resolve(
            ok({
              binding: bound,
              lastCheck: { atUnixMs: Date.now(), result: 'not_in_repository' as const },
            }),
          ),
        ),
      }),
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Check again' }));

    expect(await screen.findByText(/The room stays linked/u)).toBeVisible();
  });

  it('解绑先确认；名下已经有一间不绑仓库的公开房间时说新的限额', async () => {
    const unbind = vi.fn<ProjectRoomGateway['unbind']>(() =>
      Promise.resolve(err({ code: 'private_room.public_limit_reached', retryable: false })),
    );
    renderGroup(gateway({ binding: vi.fn(() => Promise.resolve(ok(bound))), unbind }));

    fireEvent.click(await screen.findByRole('button', { name: 'Unlink' }));
    expect(screen.getByText(/stops pointing here/u)).toBeVisible();
    expect(unbind).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Unlink' }));

    expect(
      await screen.findByText(
        /^Each account can have only one public room that isn’t linked to a repository/u,
      ),
    ).toBeVisible();
    expect(unbind).toHaveBeenCalledWith(CATALOG);
    expect(screen.getByRole('link', { name: /^Open rainyflash\/agent-room/u })).toBeVisible();
  });

  it('解绑成了：回到输入框', async () => {
    renderGroup(gateway({ binding: vi.fn(() => Promise.resolve(ok(bound))) }));

    fireEvent.click(await screen.findByRole('button', { name: 'Unlink' }));
    fireEvent.click(screen.getByRole('button', { name: 'Unlink' }));

    expect(await screen.findByRole('textbox', { name: 'Repository' })).toBeVisible();
  });
});
