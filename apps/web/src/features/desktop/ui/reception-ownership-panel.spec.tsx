// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import type { ReceptionOwnershipGateway, ReceptionRecord } from '../domain/reception-ownership';
import { ReceptionOwnershipPanel } from './reception-ownership-panel';

const calls = vi.hoisted(() => ({
  list: vi.fn<ReceptionOwnershipGateway['list']>(),
  transfer: vi.fn<ReceptionOwnershipGateway['transfer']>(),
  devices: vi.fn<AccessManagementGateway['listProductDevices']>(),
}));
vi.mock('@/app/app-services', () => ({
  useAppServices: () => ({
    receptionOwnership: { list: calls.list, transfer: calls.transfer },
    accessManagement: { listProductDevices: calls.devices },
  }),
}));
vi.mock('@/features/session/ui/session-provider', () => ({
  useSession: () => ({ snapshot: { context: { principal: { principalId: 'owner' } } } }),
}));
const record: ReceptionRecord = {
  agentId: 'agent',
  catalogId: 'catalog',
  roomId: '!room:example.org',
  sessionKey: 'session',
  displayName: 'My Agent',
  instanceId: 'instance',
  deviceId: 'computer',
  deviceLabel: 'My laptop',
  runId: 'run',
  status: 'active',
  nextDeviceId: null,
  lastSeenUnixMs: Date.now(),
  revision: 0,
  progress: { afterEventId: null, pending: null, retry: null },
};
beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});
beforeEach(() => {
  calls.list.mockReset().mockResolvedValue(ok({ receptions: [record], limited: false }));
  calls.transfer.mockReset().mockResolvedValue(ok({ ...record, status: 'draining' }));
  calls.devices.mockReset().mockResolvedValue(ok([]));
});
afterEach(cleanup);
function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <I18nextProvider i18n={i18n}>
        <ReceptionOwnershipPanel />
      </I18nextProvider>
    </QueryClientProvider>,
  );
  return client;
}
describe('reception ownership controls', () => {
  it('shows a failed device lookup and recovers through refresh', async () => {
    calls.devices.mockResolvedValueOnce(err({ code: 'network.unavailable', retryable: true }));
    const client = show();
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not load your computers');
    expect(screen.getByRole('button', { name: 'Transfer reception' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh reception status' }));
    await waitFor(() => {
      expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    });
    expect(calls.devices).toHaveBeenCalledTimes(2);
    client.clear();
  });
  it('requests a manual handover without claiming the old receiver has stopped', async () => {
    calls.list
      .mockResolvedValueOnce(ok({ receptions: [record], limited: false }))
      .mockResolvedValue(ok({ receptions: [{ ...record, status: 'draining' }], limited: false }));
    const client = show();
    fireEvent.click(await screen.findByRole('button', { name: 'Take over manually' }));
    await waitFor(() => {
      expect(calls.transfer).toHaveBeenCalledWith({
        agentId: record.agentId,
        catalogId: record.catalogId,
        nextDeviceId: null,
      });
      expect(screen.getByRole('status')).toHaveTextContent('Waiting for the previous computer');
    });
    expect(screen.getByRole('button', { name: 'Take over manually' })).toBeDisabled();
    client.clear();
  });
});
