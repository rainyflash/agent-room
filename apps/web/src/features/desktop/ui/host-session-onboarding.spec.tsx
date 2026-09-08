// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { HostSessionOnboarding } from './host-session-onboarding';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it('配置不等于接入，接入指令要求任务独立身份并解释停止监听', async () => {
  const writeText = vi.fn(() => Promise.resolve());
  vi.stubGlobal('navigator', { clipboard: { writeText } });
  render(
    <I18nextProvider i18n={i18n}>
      <HostSessionOnboarding readHostSessions={() => Promise.resolve(ok([]))} />
    </I18nextProvider>,
  );
  expect(await screen.findByText(/No agent task has opened/u)).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Copy connection instructions' }));
  expect(writeText).toHaveBeenCalledWith(expect.stringContaining('unique to this task'));
  expect(writeText).toHaveBeenCalledWith(expect.stringContaining('after this task stops'));
});

it('取信和收发证据分别展示，断线后不保留过期成功状态', async () => {
  const readHostSessions = vi.fn(() =>
    Promise.resolve(
      ok([
        {
          displayName: 'Scout',
          session: {
            sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
            state: 'ready' as const,
            agentId: null,
            errorCode: null,
          },
          lastInboxReadAgoMs: 1_000,
          lastMessageReceivedAgoMs: null,
          lastMessageSentAgoMs: 1_000,
        },
      ]),
    ),
  );
  const view = render(
    <I18nextProvider i18n={i18n}>
      <HostSessionOnboarding readHostSessions={readHostSessions} />
    </I18nextProvider>,
  );
  expect(await screen.findByText('Scout')).toBeVisible();
  expect(screen.getByText('Recently checked messages')).toBeVisible();
  expect(screen.getByText('No messages fetched yet · Message sent')).toBeVisible();
  view.rerender(
    <I18nextProvider i18n={i18n}>
      <HostSessionOnboarding
        readHostSessions={() =>
          Promise.resolve(err({ code: 'bridge.ipc.bridge_unavailable', retryable: true }))
        }
      />
    </I18nextProvider>,
  );
  expect(await screen.findByText('bridge.ipc.bridge_unavailable')).toBeVisible();
  expect(screen.queryByText('Scout')).not.toBeInTheDocument();
});
