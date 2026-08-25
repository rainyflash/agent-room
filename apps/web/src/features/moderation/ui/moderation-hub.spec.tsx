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
import { ModerationHub } from '@/features/moderation/ui/moderation-hub';
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

describe('ModerationHub', () => {
  it('只向当前房间管理者展示显式证据并绑定案件执行动作', async () => {
    const user = userEvent.setup();
    const gateway = authorizedGateway();
    renderHub(gateway);

    await user.click(await screen.findByRole('button', { name: 'Governance' }));
    expect(await screen.findByText('Reporter-submitted excerpt')).toBeVisible();
    expect(screen.getByText('Only the reporter chose this preview')).toBeVisible();

    await user.selectOptions(screen.getByLabelText('Related case ID (optional)'), CASE_ID);
    expect(screen.getByLabelText('Target reference')).toHaveValue('$event:matrix.test');
    await user.click(
      screen.getByRole('checkbox', {
        name: /I understand the target and room impact/iu,
      }),
    );
    await user.click(screen.getByRole('button', { name: 'Apply action' }));

    await waitFor(() => expect(gateway.applyAction).toHaveBeenCalledOnce());
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

  it('三种管理查询都被拒绝时不暴露治理入口', async () => {
    const gateway = forbiddenGateway();
    renderHub(gateway);

    await waitFor(() => expect(gateway.listRoomCases).toHaveBeenCalledOnce());
    await waitFor(() => expect(gateway.listActions).toHaveBeenCalledOnce());
    await waitFor(() => expect(gateway.listAudit).toHaveBeenCalledOnce());
    expect(screen.queryByRole('button', { name: 'Governance' })).not.toBeInTheDocument();
  });
});

function renderHub(gateway: ModerationGateway) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <ModerationHub
          catalogId={ROOM_ID}
          gateway={gateway}
          onReauthenticate={vi.fn()}
          recentlyAuthenticated
          roomName="Protocol Garden"
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

function authorizedGateway(): ModerationGateway {
  const moderationCase = reportCase();
  const action = appliedAction();
  return {
    applyAction: vi.fn(() => Promise.resolve(ok(action))),
    listActions: vi.fn(() => Promise.resolve(ok([]))),
    listAudit: vi.fn(() => Promise.resolve(ok([]))),
    listCases: vi.fn(() => Promise.resolve(ok([moderationCase]))),
    listRoomCases: vi.fn(() => Promise.resolve(ok([moderationCase]))),
    report: vi.fn(() => Promise.resolve(ok(moderationCase))),
    reverseAction: vi.fn(() => Promise.resolve(ok(action))),
  };
}

function forbiddenGateway(): ModerationGateway {
  const forbidden = {
    code: 'moderation.forbidden',
    retryable: false,
  } as const;
  return {
    applyAction: vi.fn(() => Promise.resolve(err(forbidden))),
    listActions: vi.fn(() => Promise.resolve(err(forbidden))),
    listAudit: vi.fn(() => Promise.resolve(err(forbidden))),
    listCases: vi.fn(() => Promise.resolve(err(forbidden))),
    listRoomCases: vi.fn(() => Promise.resolve(err(forbidden))),
    report: vi.fn(() => Promise.resolve(err(forbidden))),
    reverseAction: vi.fn(() => Promise.resolve(err(forbidden))),
  };
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
