// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  AgentRecoveryGateway,
  AgentRecoveryResult,
} from '@/features/security/domain/agent-recovery';
import { AgentRecoveryPanel } from '@/features/security/ui/agent-recovery-panel';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const sessionId = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const secondId = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const initial: AgentRecoveryResult = {
  state: {
    userId: '@builder:example.test',
    deviceId: 'BUILDER',
    identity: 'ready',
    recoveryAvailable: false,
    backupEnabled: false,
  },
  recoveryKey: null,
};
beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(cleanup);

function show(execute: AgentRecoveryGateway['execute'], empty = false) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <AgentRecoveryPanel
          gateway={{
            execute,
            sessions: () =>
              Promise.resolve(
                ok(
                  empty
                    ? []
                    : [
                        { sessionId, displayName: 'Builder', state: 'ready' },
                        { sessionId: secondId, displayName: 'Scout', state: 'ready' },
                      ],
                ),
              ),
          }}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return queryClient;
}

describe('Agent 恢复面板', () => {
  it('确认口令后只设置当前 Agent，密钥不会进入查询或 mutation 缓存', async () => {
    const user = userEvent.setup();
    const key = 'one-time-agent-recovery-key';
    const execute = vi.fn<AgentRecoveryGateway['execute']>((_session, request) =>
      Promise.resolve(ok(request.action === 'enable' ? { ...initial, recoveryKey: key } : initial)),
    );
    const queryClient = show(execute);
    await user.click(await screen.findByRole('button', { name: 'Set up recovery' }));
    await user.type(screen.getByLabelText('Recovery passphrase'), 'a long test passphrase');
    await user.type(screen.getByLabelText('Confirm passphrase'), 'mismatch');
    expect(screen.getByRole('button', { name: 'Create recovery' })).toBeDisabled();
    await user.clear(screen.getByLabelText('Confirm passphrase'));
    await user.type(screen.getByLabelText('Confirm passphrase'), 'a long test passphrase');
    expect(screen.getByRole('button', { name: 'Refresh security state' })).toBeDisabled();
    act(() => {
      queryClient.setQueryData(
        ['local-agent-recovery-sessions'],
        ok([
          { sessionId: secondId, displayName: 'Scout', state: 'ready' },
          { sessionId, displayName: 'Builder', state: 'ready' },
        ]),
      );
    });
    await user.click(screen.getByRole('button', { name: 'Create recovery' }));
    expect(await screen.findByText(key)).toBeVisible();
    expect(execute).toHaveBeenCalledWith(sessionId, {
      action: 'enable',
      passphrase: 'a long test passphrase',
    });
    expect(screen.getByRole('combobox')).toBeDisabled();
    await waitFor(() => {
      expect(queryClient.isMutating()).toBe(0);
    });
    const cached = JSON.stringify([
      queryClient
        .getQueryCache()
        .getAll()
        .map((q) => q.state),
      queryClient
        .getMutationCache()
        .getAll()
        .map((m) => m.state),
    ]);
    expect(cached).not.toContain(key);
    expect(cached).not.toContain('a long test passphrase');
    await user.click(screen.getByRole('button', { name: 'I saved the recovery key' }));
    expect(screen.queryByText(key)).not.toBeInTheDocument();
    expect(screen.getByRole('combobox')).toBeEnabled();
  });

  it('错误恢复口令被清空，切换 Agent 不携带旧表单', async () => {
    const user = userEvent.setup();
    const locked: AgentRecoveryResult = {
      ...initial,
      state: { ...initial.state, identity: 'recovery_required', recoveryAvailable: true },
    };
    const execute = vi.fn<AgentRecoveryGateway['execute']>((_session, request) =>
      Promise.resolve(
        request.action === 'restore'
          ? err({ code: 'bridge.security.recovery_rejected', retryable: false })
          : ok(locked),
      ),
    );
    show(execute);
    await user.click(await screen.findByRole('button', { name: 'Unlock Agent recovery' }));
    await user.type(screen.getByLabelText('Passphrase or recovery key'), 'wrong credential');
    await user.click(screen.getByRole('button', { name: 'Recover history' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The passphrase or recovery key was rejected.',
    );
    expect(screen.getByLabelText('Passphrase or recovery key')).toHaveValue('');
    await user.type(screen.getByLabelText('Passphrase or recovery key'), 'do not carry over');
    await user.selectOptions(screen.getByRole('combobox'), secondId);
    await user.click(await screen.findByRole('button', { name: 'Unlock Agent recovery' }));
    expect(screen.getByLabelText('Passphrase or recovery key')).toHaveValue('');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('没有运行中 Agent 时刷新不会发送空会话命令', async () => {
    const user = userEvent.setup();
    const execute = vi.fn<AgentRecoveryGateway['execute']>(() => Promise.resolve(ok(initial)));
    show(execute, true);
    await screen.findByText(/No running Agent is available/u);
    await user.click(screen.getByRole('button', { name: 'Refresh security state' }));
    expect(execute).not.toHaveBeenCalled();
  });
});
