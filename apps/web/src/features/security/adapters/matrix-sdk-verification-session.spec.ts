import {
  VerificationPhase,
  VerificationRequestEvent,
  VerifierEvent,
  type ShowSasCallbacks,
  type VerificationRequest,
  type Verifier,
} from 'matrix-js-sdk/lib/crypto-api/index.js';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkVerificationSession } from '@/features/security/adapters/matrix-sdk-verification-session';

describe('MatrixSdkVerificationSession', () => {
  it('只在双方确认同一组 SAS 后进入已验证状态', async () => {
    const flow = verificationFlow();
    const session = new MatrixSdkVerificationSession(flow.request, 'OTHER-DEVICE');
    session.activate();

    flow.advance(VerificationPhase.Ready);
    await vi.waitFor(() => {
      expect(flow.startVerification).toHaveBeenCalledWith('m.sas.v1');
    });
    flow.showSas();

    expect(session.getSnapshot()).toEqual({
      sas: {
        decimals: [1024, 2048, 4096],
        emojis: [
          { label: 'Dog', symbol: '🐶' },
          { label: 'Rocket', symbol: '🚀' },
        ],
      },
      stage: 'comparing',
    });
    await expect(session.confirm()).resolves.toEqual({ ok: true, value: undefined });
    expect(flow.confirm).toHaveBeenCalledOnce();
    expect(session.getSnapshot()).toMatchObject({ stage: 'confirming' });

    flow.complete();
    await vi.waitFor(() => {
      expect(session.getSnapshot()).toEqual({ stage: 'verified' });
    });
  });

  it('SAS 不一致会终止事务且不会提供强制信任旁路', async () => {
    const flow = verificationFlow();
    const session = new MatrixSdkVerificationSession(flow.request);
    session.activate();
    flow.advance(VerificationPhase.Ready);
    await vi.waitFor(() => {
      expect(flow.startVerification).toHaveBeenCalledOnce();
    });
    flow.showSas();

    session.mismatch();

    expect(flow.mismatch).toHaveBeenCalledOnce();
    expect(session.getSnapshot()).toEqual({
      cancellationCode: 'm.mismatched_sas',
      stage: 'cancelled',
    });
  });

  it('请求端在 Ready 后等待接受端选择 SAS，避免双方争抢 start 事件', () => {
    const flow = verificationFlow();
    const session = new MatrixSdkVerificationSession(flow.request, 'OTHER-DEVICE', false);
    session.activate();

    flow.advance(VerificationPhase.Ready);
    expect(flow.startVerification).not.toHaveBeenCalled();

    flow.advance(VerificationPhase.Started);
    flow.showSas();

    expect(session.getSnapshot()).toMatchObject({ stage: 'comparing' });
    expect(flow.startVerification).not.toHaveBeenCalled();
    session.deactivate();
  });

  it('接受端确认请求后选择 SAS 并启动 verifier', async () => {
    const flow = verificationFlow();
    const session = new MatrixSdkVerificationSession(flow.request, 'OTHER-DEVICE');
    session.activate();

    flow.advance(VerificationPhase.Ready);

    await vi.waitFor(() => {
      expect(flow.startVerification).toHaveBeenCalledWith('m.sas.v1');
    });
    session.deactivate();
  });

  it('SDK 漏发请求变更事件时仍会对账并挂载远端创建的 verifier', async () => {
    vi.useFakeTimers();
    const flow = verificationFlow();
    const session = new MatrixSdkVerificationSession(flow.request, 'OTHER-DEVICE', false);

    try {
      session.activate();
      flow.advance(VerificationPhase.Ready);
      flow.advanceSilently(VerificationPhase.Started);

      expect(flow.verify).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(250);

      expect(flow.verify).toHaveBeenCalledOnce();
    } finally {
      session.deactivate();
      vi.useRealTimers();
    }
  });

  it('React StrictMode 预演停用后可重新激活同一验证会话', () => {
    const flow = verificationFlow();
    const session = new MatrixSdkVerificationSession(flow.request, 'OTHER-DEVICE', false);

    session.activate();
    session.deactivate();
    session.activate();
    flow.advance(VerificationPhase.Ready);
    flow.advance(VerificationPhase.Started);
    flow.showSas();

    expect(flow.verify).toHaveBeenCalledOnce();
    expect(session.getSnapshot()).toMatchObject({ stage: 'comparing' });
    session.deactivate();
  });
});

function verificationFlow() {
  const requestListeners = new Set<() => void>();
  const sasListeners = new Set<(callbacks: ShowSasCallbacks) => void>();
  let phase = VerificationPhase.Requested;
  let resolveVerification: (() => void) | undefined;
  const verification = new Promise<void>((resolve) => {
    resolveVerification = resolve;
  });
  const confirm = vi.fn(() => Promise.resolve());
  const mismatch = vi.fn();
  const sasCallbacks: ShowSasCallbacks = {
    cancel: vi.fn(),
    confirm,
    mismatch,
    sas: {
      decimal: [1024, 2048, 4096],
      emoji: [
        ['🐶', 'Dog'],
        ['🚀', 'Rocket'],
      ],
    },
  };
  const verify = vi.fn(() => verification);
  const verifierShape = {
    get hasBeenCancelled() {
      return false;
    },
    getShowSasCallbacks: () => null,
    off: (event: VerifierEvent, listener: unknown) => {
      if (event === VerifierEvent.ShowSas) {
        sasListeners.delete(listener as (callbacks: ShowSasCallbacks) => void);
      }
      return verifierShape;
    },
    on: (event: VerifierEvent, listener: unknown) => {
      if (event === VerifierEvent.ShowSas) {
        sasListeners.add(listener as (callbacks: ShowSasCallbacks) => void);
      }
      return verifierShape;
    },
    verify,
  };
  const verifier = verifierShape as unknown as Verifier;
  const startVerification = vi.fn(() => Promise.resolve(verifier));
  const requestShape = {
    cancellationCode: null,
    cancel: () => Promise.resolve(),
    get phase() {
      return phase;
    },
    startVerification,
    transactionId: 'verification-transaction',
    get verifier() {
      return phase === VerificationPhase.Started ? verifier : undefined;
    },
    off: (_event: VerificationRequestEvent, listener: unknown) => {
      requestListeners.delete(listener as () => void);
      return requestShape;
    },
    on: (_event: VerificationRequestEvent, listener: unknown) => {
      requestListeners.add(listener as () => void);
      return requestShape;
    },
  };
  const request = requestShape as unknown as VerificationRequest;

  return {
    advance: (next: VerificationPhase) => {
      phase = next;
      for (const listener of requestListeners) {
        listener();
      }
    },
    advanceSilently: (next: VerificationPhase) => {
      phase = next;
    },
    complete: () => resolveVerification?.(),
    confirm,
    mismatch,
    request,
    showSas: () => {
      for (const listener of sasListeners) {
        listener(sasCallbacks);
      }
    },
    startVerification,
    verify,
  };
}
