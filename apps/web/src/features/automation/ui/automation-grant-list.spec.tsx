// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { AutomationGrant } from '@/features/automation/domain/automation-grant';
import { AutomationGrantList } from '@/features/automation/ui/automation-grant-list';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(cleanup);

describe('AutomationGrantList', () => {
  it('计数只算有效授权，失效的收进折叠区', () => {
    renderList([live(), revoked()]);

    expect(screen.getByText('1 grant')).toBeInTheDocument();
    expect(screen.getByText('1 past grant')).toBeInTheDocument();
    expect(screen.getByRole('group')).not.toHaveAttribute('open');
    expect(screen.getAllByRole('button', { name: 'Revoke' })).toHaveLength(1);
  });

  it('刚失效的授权会把折叠区展开，撤销后不至于没有回执', () => {
    const view = renderList([live()]);
    expect(screen.queryByRole('group')).not.toBeInTheDocument();

    view.rerender(list([revoked()]));

    expect(screen.getByRole('group')).toHaveAttribute('open');
    expect(screen.queryByRole('button', { name: 'Revoke' })).not.toBeInTheDocument();
  });

  it('只有历史授权时正文说明当前没有生效的授权', () => {
    renderList([revoked()]);

    expect(screen.getByText('No automation grant is active for this room.')).toBeInTheDocument();
  });
});

function renderList(grants: readonly AutomationGrant[]) {
  return render(list(grants));
}

function list(grants: readonly AutomationGrant[]) {
  return (
    <I18nextProvider i18n={i18n}>
      <AutomationGrantList
        grants={grants}
        instances={[]}
        onRevoke={vi.fn()}
        pendingGrantId={null}
      />
    </I18nextProvider>
  );
}

function live(): AutomationGrant {
  return grant('40', {});
}

function revoked(): AutomationGrant {
  return grant('41', { revokedAtUnixMs: Date.now() - 1_000, status: 'revoked' });
}

function grant(suffix: string, overrides: Partial<AutomationGrant>): AutomationGrant {
  return {
    agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
    agentInstanceId: null,
    audience: 'known_room_members',
    expiresAtUnixMs: Date.now() + 600_000,
    grantId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${suffix}`,
    maxMessagesPerMinute: 6,
    maxTotalMessages: null,
    messageKinds: ['reply'],
    messagesInCurrentMinute: 0,
    requiresRiskScan: true,
    revokedAtUnixMs: null,
    roomCatalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
    startsAtUnixMs: Date.now() - 60_000,
    status: 'active',
    totalMessages: 0,
    ...overrides,
  };
}
