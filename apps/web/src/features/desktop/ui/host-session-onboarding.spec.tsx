// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { HostSessionOnboarding } from './host-session-onboarding';
import { DesktopRuntimeProvider } from './desktop-runtime-provider';
import type { DesktopRuntimeGateway } from '../domain/desktop-runtime';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const router = vi.hoisted(() => ({ navigate: vi.fn() }));
vi.mock('@tanstack/react-router', async (loadOriginal) => {
  const original = await loadOriginal<typeof import('@tanstack/react-router')>();
  return { ...original, useNavigate: () => router.navigate };
});

const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
const gateway: DesktopRuntimeGateway = {
  beginHumanAuthentication: unavailable,
  beginMatrixAuthentication: unavailable,
  bootstrapDefaultAgent: unavailable,
  checkUpdate: unavailable,
  clearHumanSession: () => Promise.resolve(ok(undefined)),
  restoreHumanSession: () => Promise.resolve(ok(true)),
  configureAgentRuntime: (target) => Promise.resolve(ok(target)),
  installUpdate: unavailable,
  isAvailable: () => false,
  openAuthorization: unavailable,
  readLobby: unavailable,
  retryBridge: unavailable,
  setAutostart: (enabled) => Promise.resolve(ok(enabled)),
  snapshot: unavailable,
  subscribe: () => Promise.resolve(ok(() => undefined)),
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it('配置不等于接入：空清单时提供打开接入面板的入口', async () => {
  render(
    <I18nextProvider i18n={i18n}>
      <DesktopRuntimeProvider gateway={gateway}>
        <HostSessionOnboarding readHostSessions={() => Promise.resolve(ok([]))} />
      </DesktopRuntimeProvider>
    </I18nextProvider>,
  );
  expect(await screen.findByText(/No agent task has opened/u)).toBeVisible();
  expect(screen.queryByRole('dialog')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Bring an agent' }));
  expect(screen.getByRole('dialog', { name: 'Bring an agent into the room' })).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Close' }));
  expect(screen.queryByRole('dialog')).toBeNull();
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
      <DesktopRuntimeProvider gateway={gateway}>
        <HostSessionOnboarding readHostSessions={readHostSessions} />
      </DesktopRuntimeProvider>
    </I18nextProvider>,
  );
  expect(await screen.findByText('Scout')).toBeVisible();
  expect(screen.getByText('Recently checked messages')).toBeVisible();
  expect(screen.getByText('No messages fetched yet · Message sent')).toBeVisible();
  view.rerender(
    <I18nextProvider i18n={i18n}>
      <DesktopRuntimeProvider gateway={gateway}>
        <HostSessionOnboarding
          readHostSessions={() =>
            Promise.resolve(err({ code: 'bridge.ipc.bridge_unavailable', retryable: true }))
          }
        />
      </DesktopRuntimeProvider>
    </I18nextProvider>,
  );
  expect(await screen.findByText('bridge.ipc.bridge_unavailable')).toBeVisible();
  expect(screen.queryByText('Scout')).not.toBeInTheDocument();
});
