// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  ModerationAction,
  ModerationCase,
  ModerationGateway,
} from '@/features/moderation/domain/moderation';
import { ModerationSettings } from '@/features/moderation/ui/moderation-settings';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const ROOM_ID = '01990d9e-8400-7000-8000-000000000031';
const CASE_ID = '01990d9e-8400-7000-8000-000000000032';
const ACTION_ID = '01990d9e-8400-7000-8000-000000000033';
const ACTOR_ID = '01990d9e-8400-7000-8000-000000000034';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('ModerationSettings', () => {
  it('只向当前房间管理者展示显式证据并绑定案件执行动作', async () => {
    const user = userEvent.setup();
    const gateway = authorizedGateway();
    renderSettings(gateway);

    expect(await screen.findByText('Summary included by the reporter')).toBeVisible();
    expect(screen.getByText('Only the reporter chose this preview')).toBeVisible();

    await user.selectOptions(screen.getByLabelText('Which report (optional)'), CASE_ID);
    expect(screen.getByLabelText('Who or what')).toHaveValue('$event:matrix.test');
    await user.click(
      screen.getByRole('checkbox', {
        name: /I understand who this affects/iu,
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Apply action' }));

    await waitFor(() => {
      expect(gateway.applyAction).toHaveBeenCalledOnce();
    });
    expect(gateway.applyAction).toHaveBeenCalledWith(
      expect.any(String),
      ROOM_ID,
      expect.objectContaining({
        caseId: CASE_ID,
        impactAcknowledged: true,
        kind: 'hide',
        targetKind: 'event',
        targetReference: '$event:matrix.test',
      }),
    );
  });

  it('撤回禁言回冲突时说清楚过一会儿再点，不说读不到治理信息', async () => {
    const user = userEvent.setup();
    const mute: ModerationAction = {
      ...appliedAction(),
      kind: 'mute',
      targetKind: 'principal',
      targetReference: ACTOR_ID,
    };
    const gateway = {
      ...authorizedGateway(),
      listActions: vi.fn(() => Promise.resolve(ok([mute]))),
      reverseAction: vi.fn(() =>
        Promise.resolve(err({ code: 'moderation.conflict', retryable: false } as const)),
      ),
    } satisfies ModerationGateway;
    renderSettings(gateway);

    await user.click(await screen.findByRole('button', { name: 'Undo' }));

    expect(await screen.findByText(/^Can’t undo right now/u)).toBeVisible();
    expect(screen.queryByText(/Could not load moderation/u)).toBeNull();
    expect(gateway.reverseAction).toHaveBeenCalledWith(ACTION_ID);
  });

  it('到了期限不用刷新就改说正在解除，正在解除时隔 15 秒再读一次台账', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const mute: ModerationAction = {
        ...appliedAction(),
        expiresAtUnixMs: Date.now() + 60_000,
        kind: 'mute',
        startsAtUnixMs: Date.now() - 1_000,
        targetKind: 'principal',
        targetReference: ACTOR_ID,
      };
      const gateway = {
        ...authorizedGateway(),
        listActions: vi.fn(() => Promise.resolve(ok([mute]))),
      } satisfies ModerationGateway;
      renderSettings(gateway);
      expect(await screen.findByRole('img', { name: 'Applied' })).toBeVisible();

      await vi.advanceTimersByTimeAsync(60_000);
      expect(await screen.findByRole('img', { name: 'Ending' })).toBeVisible();

      const reads = gateway.listActions.mock.calls.length;
      await vi.advanceTimersByTimeAsync(15_000);
      await waitFor(() => {
        expect(gateway.listActions.mock.calls.length).toBeGreaterThan(reads);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('普通成员只读取能力投影且不探测任何受限治理资源', async () => {
    const gateway = unauthorizedGateway();
    const { container } = renderSettings(gateway);

    await waitFor(() => {
      expect(gateway.inspectCapabilities).toHaveBeenCalledOnce();
    });
    expect(gateway.listRoomCases).not.toHaveBeenCalled();
    expect(gateway.listActions).not.toHaveBeenCalled();
    expect(gateway.listAudit).not.toHaveBeenCalled();
    expect(container).toBeEmptyDOMElement();
  });
});

function renderSettings(gateway: ModerationGateway) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <ModerationSettings
          catalogId={ROOM_ID}
          gateway={gateway}
          onReauthenticate={vi.fn()}
          recentlyAuthenticated
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function authorizedGateway() {
  const moderationCase = reportCase();
  const action = appliedAction();
  return {
    applyAction: vi.fn(() => Promise.resolve(ok(action))),
    inspectCapabilities: vi.fn(() =>
      Promise.resolve(ok({ canModerateRoom: true, canReadAudit: true })),
    ),
    listActions: vi.fn(() => Promise.resolve(ok([]))),
    listAudit: vi.fn(() => Promise.resolve(ok([]))),
    listCases: vi.fn(() => Promise.resolve(ok([moderationCase]))),
    listRoomCases: vi.fn(() => Promise.resolve(ok([moderationCase]))),
    report: vi.fn(() => Promise.resolve(ok(moderationCase))),
    reverseAction: vi.fn(() => Promise.resolve(ok(action))),
  } satisfies ModerationGateway;
}

function unauthorizedGateway() {
  const forbidden = {
    code: 'moderation.forbidden',
    retryable: false,
  } as const;
  return {
    applyAction: vi.fn(() => Promise.resolve(err(forbidden))),
    inspectCapabilities: vi.fn(() =>
      Promise.resolve(ok({ canModerateRoom: false, canReadAudit: false })),
    ),
    listActions: vi.fn(() => Promise.resolve(err(forbidden))),
    listAudit: vi.fn(() => Promise.resolve(err(forbidden))),
    listCases: vi.fn(() => Promise.resolve(err(forbidden))),
    listRoomCases: vi.fn(() => Promise.resolve(err(forbidden))),
    report: vi.fn(() => Promise.resolve(err(forbidden))),
    reverseAction: vi.fn(() => Promise.resolve(err(forbidden))),
  } satisfies ModerationGateway;
}

function reportCase(): ModerationCase {
  return {
    caseId: CASE_ID,
    createdAtUnixMs: 1_800_000_000_000,
    description: 'Only the facts required for review',
    evidence: {
      endToEndEncrypted: true,
      matrixEventId: '$event:matrix.test',
      reporterSubmittedExcerpt: 'Only the reporter chose this preview',
      roomCatalogId: ROOM_ID,
    },
    reason: 'harassment',
    resolvedAtUnixMs: null,
    state: 'open',
    targetKind: 'event',
    targetReference: '$event:matrix.test',
  };
}

function appliedAction(): ModerationAction {
  return {
    actionId: ACTION_ID,
    actorPrincipalId: ACTOR_ID,
    caseId: CASE_ID,
    expiresAtUnixMs: null,
    failureCode: null,
    kind: 'hide',
    reason: 'harassment',
    reversedAtUnixMs: null,
    roomCatalogId: ROOM_ID,
    startsAtUnixMs: 1_800_000_000_100,
    status: 'applied',
    targetKind: 'event',
    targetReference: '$event:matrix.test',
  };
}
