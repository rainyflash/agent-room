// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { endingRefetchInterval } from '@/features/moderation/data/moderation-queries';
import {
  moderationActionDisplayStatus,
  nextModerationExpiry,
  type ModerationAction,
} from '@/features/moderation/domain/moderation';
import { ModerationActionLedger } from '@/features/moderation/ui/moderation-ledger';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const ROOM_ID = '01990d9e-8400-7000-8000-000000000041';
const ACTOR_ID = '01990d9e-8400-7000-8000-000000000042';
const STARTS_AT = 1_800_000_000_000;
const HOUR = 3_600_000;
/** 禁言一小时，过了半小时。 */
const HALF_WAY = STARTS_AT + HOUR / 2;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('moderationActionDisplayStatus', () => {
  it('撤销时间不早于到期时间的算到期解除，别的照服务器给的', () => {
    const later = STARTS_AT + 2 * HOUR;
    expect(moderationActionDisplayStatus(expiredMute(), later)).toBe('expired');
    expect(
      moderationActionDisplayStatus(
        mute({ reversedAtUnixMs: STARTS_AT + HOUR + 30_000, status: 'reversed' }),
        later,
      ),
    ).toBe('expired');
    expect(moderationActionDisplayStatus(undoneMute(), later)).toBe('reversed');
    expect(
      moderationActionDisplayStatus(
        mute({
          expiresAtUnixMs: null,
          reversedAtUnixMs: STARTS_AT + 2 * HOUR,
          status: 'reversed',
        }),
        later,
      ),
    ).toBe('reversed');
    expect(moderationActionDisplayStatus(mute({}), HALF_WAY)).toBe('applied');
  });

  it('过了期限、服务器还没解除的算正在解除，没有期限的一直生效', () => {
    expect(moderationActionDisplayStatus(mute({}), STARTS_AT + HOUR - 1)).toBe('applied');
    expect(moderationActionDisplayStatus(mute({}), STARTS_AT + HOUR)).toBe('ending');
    expect(
      moderationActionDisplayStatus(mute({ expiresAtUnixMs: null }), STARTS_AT + 2 * HOUR),
    ).toBe('applied');
    expect(moderationActionDisplayStatus(mute({ status: 'failed' }), STARTS_AT + 2 * HOUR)).toBe(
      'failed',
    );
  });
});

describe('nextModerationExpiry', () => {
  it('已生效的里面下一个到期的，过了的、撤了的、没期限的都不算', () => {
    const actions = [
      mute({ actionId: actionId(1), expiresAtUnixMs: STARTS_AT + 3 * HOUR }),
      mute({ actionId: actionId(2), expiresAtUnixMs: STARTS_AT + 2 * HOUR }),
      mute({ actionId: actionId(3), expiresAtUnixMs: STARTS_AT + HOUR }),
      undoneMute(),
      mute({ actionId: actionId(4), expiresAtUnixMs: null }),
    ];
    expect(nextModerationExpiry(actions, HALF_WAY)).toBe(STARTS_AT + HOUR);
    expect(nextModerationExpiry(actions, STARTS_AT + HOUR)).toBe(STARTS_AT + 2 * HOUR);
    expect(nextModerationExpiry(actions, STARTS_AT + 3 * HOUR)).toBeNull();
  });
});

describe('endingRefetchInterval', () => {
  it('有正在解除的才隔一会儿再读，读不到、都定了就不读', () => {
    const due = STARTS_AT + HOUR;
    expect(endingRefetchInterval(ok([mute({})]), due - 1)).toBe(false);
    expect(endingRefetchInterval(ok([mute({})]), due)).toBe(15_000);
    expect(endingRefetchInterval(ok([expiredMute()]), due + HOUR)).toBe(false);
    expect(
      endingRefetchInterval(err({ code: 'moderation.unavailable', retryable: true }), due),
    ).toBe(false);
    expect(endingRefetchInterval(undefined, due)).toBe(false);
  });
});

describe('ModerationActionLedger', () => {
  it('到期解除的和人工撤回的分开说，限时生效中的说什么时候自动解除', () => {
    renderLedger([
      mute({ actionId: actionId(1) }),
      expiredMute(),
      undoneMute(),
      mute({ actionId: actionId(4), expiresAtUnixMs: null, kind: 'ban' }),
    ]);
    expect(screen.getAllByRole('listitem')).toHaveLength(4);
    const active = ledgerRow(0);
    const expired = ledgerRow(1);
    const undone = ledgerRow(2);
    const indefinite = ledgerRow(3);

    expect(active.getByRole('img', { name: 'Applied' })).toBeVisible();
    expect(active.getByText(/^Ends automatically at /u)).toBeVisible();
    expect(active.getByRole('button', { name: 'Undo' })).toBeEnabled();

    expect(expired.getByRole('img', { name: 'Expired' })).toBeVisible();
    expect(expired.getByText(/^Expired at /u)).toBeVisible();
    expect(expired.queryByRole('button', { name: 'Undo' })).toBeNull();

    expect(undone.getByRole('img', { name: 'Undone' })).toBeVisible();
    expect(undone.queryByText(/Expired at|Ends automatically/u)).toBeNull();

    expect(indefinite.getByRole('img', { name: 'Applied' })).toBeVisible();
    expect(indefinite.queryByText(/Ends automatically/u)).toBeNull();
  });

  it('过了期限、服务器还没解除的说正在解除，写到期时间，撤回还能点', () => {
    renderLedger([mute({})], STARTS_AT + HOUR + 60_000);
    const ending = ledgerRow(0);

    expect(ending.getByRole('img', { name: 'Ending' })).toBeVisible();
    expect(ending.getByText(/^Expired at /u)).toBeVisible();
    expect(ending.queryByText(/Ends automatically/u)).toBeNull();
    expect(ending.getByRole('button', { name: 'Undo' })).toBeEnabled();
  });

  it('中文说“到期解除”', async () => {
    await i18n.changeLanguage('zh-CN');
    try {
      renderLedger([expiredMute(), mute({})], STARTS_AT + HOUR + 60_000);
      expect(ledgerRow(0).getByRole('img', { name: '到期解除' })).toBeVisible();
      expect(ledgerRow(0).getByText(/ 到期$/u)).toBeVisible();
      expect(ledgerRow(1).getByRole('img', { name: '正在解除' })).toBeVisible();
      expect(ledgerRow(1).getByText(/ 到期$/u)).toBeVisible();
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});

function renderLedger(actions: readonly ModerationAction[], now = HALF_WAY) {
  return render(
    <I18nextProvider i18n={i18n}>
      <ModerationActionLedger
        actions={actions}
        now={now}
        onReverse={vi.fn()}
        pendingActionId={null}
        recentlyAuthenticated
      />
    </I18nextProvider>,
  );
}

function ledgerRow(index: number) {
  const row = screen.getAllByRole('listitem')[index];
  if (row === undefined) throw new Error(`ledger row ${String(index)} missing`);
  return within(row);
}

/** 禁言一小时，到期时由服务器解除。 */
function expiredMute(): ModerationAction {
  return mute({
    actionId: actionId(2),
    reversedAtUnixMs: STARTS_AT + HOUR,
    status: 'reversed',
  });
}

/** 禁言一小时，十分钟后被管理员撤回。 */
function undoneMute(): ModerationAction {
  return mute({
    actionId: actionId(3),
    reversedAtUnixMs: STARTS_AT + 600_000,
    status: 'reversed',
  });
}

function mute(overrides: Partial<ModerationAction>): ModerationAction {
  return {
    actionId: actionId(9),
    actorPrincipalId: ACTOR_ID,
    caseId: null,
    expiresAtUnixMs: STARTS_AT + HOUR,
    failureCode: null,
    kind: 'mute',
    reason: 'spam',
    reversedAtUnixMs: null,
    roomCatalogId: ROOM_ID,
    startsAtUnixMs: STARTS_AT,
    status: 'applied',
    targetKind: 'principal',
    targetReference: '01990d9e-8400-7000-8000-000000000043',
    ...overrides,
  };
}

function actionId(index: number): string {
  return `01990d9e-8400-7000-8000-${String(index).padStart(12, '0')}`;
}
