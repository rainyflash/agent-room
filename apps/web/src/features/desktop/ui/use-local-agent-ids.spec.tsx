// @vitest-environment jsdom

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import type { PropsWithChildren } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AppServicesProvider, type AppServices } from '@/app/app-services';
import type {
  DesktopRuntimeGateway,
  HostSessionDiagnostics,
} from '@/features/desktop/domain/desktop-runtime';
import { err, ok } from '@/shared/result';

import { useLocalAgentIds } from './use-local-agent-ids';

const FIRST = '01990d9e-8400-7000-8000-000000000001';
const SECOND = '01990d9e-8400-7000-8000-000000000002';

function session(index: number, agentId: string | null): HostSessionDiagnostics {
  return {
    displayName: `Agent ${String(index)}`,
    lastInboxReadAgoMs: null,
    lastMessageReceivedAgoMs: null,
    lastMessageSentAgoMs: null,
    session: {
      agentId,
      errorCode: null,
      sessionId: `01990d9e-8400-7000-8000-0000000001${String(index).padStart(2, '0')}`,
      state: agentId === null ? 'starting' : 'ready',
    },
  };
}

type ReadHostSessions = NonNullable<DesktopRuntimeGateway['readHostSessions']>;

function renderIds(
  enabled: boolean,
  runtime: { readonly available: boolean; readonly readHostSessions?: ReadHostSessions },
) {
  const services = {
    localRuntime: {
      isAvailable: () => runtime.available,
      ...(runtime.readHostSessions === undefined
        ? {}
        : { readHostSessions: runtime.readHostSessions }),
    },
  } as unknown as AppServices;
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>
      <AppServicesProvider services={services}>{children}</AppServicesProvider>
    </QueryClientProvider>
  );
  return renderHook(() => useLocalAgentIds(enabled), { wrapper });
}

afterEach(cleanup);

describe('useLocalAgentIds', () => {
  it('回这台电脑的连接服务记着的会话里的 Agent；还没登记上 Agent 的会话不算', async () => {
    const readHostSessions = vi.fn<ReadHostSessions>(() =>
      Promise.resolve(ok([session(1, FIRST), session(2, null), session(3, SECOND)])),
    );
    const { result } = renderIds(true, { available: true, readHostSessions });

    // 还没读到时是“不知道”，不是“一个都没有”。
    expect(result.current).toBeNull();
    await waitFor(() => {
      expect(result.current).not.toBeNull();
    });
    expect([...(result.current ?? [])]).toEqual([FIRST, SECOND]);
  });

  it('浏览器里没有连接服务：不去问，一直是不知道', () => {
    const readHostSessions = vi.fn<ReadHostSessions>(() => Promise.resolve(ok([])));
    const { result } = renderIds(true, { available: false, readHostSessions });

    expect(result.current).toBeNull();
    expect(readHostSessions).not.toHaveBeenCalled();
  });

  it('用不着的时候不去问', () => {
    const readHostSessions = vi.fn<ReadHostSessions>(() => Promise.resolve(ok([])));
    const { result } = renderIds(false, { available: true, readHostSessions });

    expect(result.current).toBeNull();
    expect(readHostSessions).not.toHaveBeenCalled();
  });

  it('读不出来、旧的桌面端没有这个命令：也是不知道', async () => {
    const failing = vi.fn<ReadHostSessions>(() =>
      Promise.resolve(err({ code: 'desktop.bridge.unavailable', retryable: true })),
    );
    const failed = renderIds(true, { available: true, readHostSessions: failing });
    await waitFor(() => {
      expect(failing).toHaveBeenCalled();
    });
    expect(failed.result.current).toBeNull();

    const missing = renderIds(true, { available: true });
    expect(missing.result.current).toBeNull();
  });
});
