import type { Logger } from 'matrix-js-sdk/lib/logger.js';
// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { MatrixLifecycleLogger } from './matrix-lifecycle-logger';

function sink() {
  const output: Logger = {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    getChild: () => output,
  };
  return output;
}

/** jsdom 没有 PromiseRejectionEvent，造一个长得一样的；返回有没有被当成已处理。 */
function rejectUnhandled(reason: unknown): boolean {
  const event = Object.assign(new Event('unhandledrejection', { cancelable: true }), { reason });
  window.dispatchEvent(event);
  return event.defaultPrevented;
}

describe('Matrix shutdown logging', () => {
  it('退出留下的、没人接的 AbortError 只记成调试信息，别的错和过了这段时间的照常报', () => {
    // Alpha 88 的发布 CI 红在这里：退出落在启动同步的半路，SDK 发完第一个同步请求就不管了。
    vi.useFakeTimers();
    try {
      const output = sink();
      const lifecycle = new MatrixLifecycleLogger(output);
      const abort = new DOMException('signal is aborted without reason', 'AbortError');

      expect(rejectUnhandled(abort)).toBe(false);

      lifecycle.beginShutdown();
      lifecycle.beginShutdown();
      expect(rejectUnhandled(abort)).toBe(true);
      expect(output.debug).toHaveBeenCalledTimes(1);
      expect(output.debug).toHaveBeenCalledWith('Abandoned Matrix request after shutdown', abort);
      expect(rejectUnhandled(new Error('Server rejected logout'))).toBe(false);

      vi.advanceTimersByTime(10_000);
      expect(rejectUnhandled(abort)).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps verbose SDK trace output disabled by default', () => {
    const trace = vi.spyOn(console, 'trace').mockImplementation(() => undefined);
    try {
      const lifecycle = new MatrixLifecycleLogger();
      lifecycle.logger.getChild('crypto').trace('verbose diagnostic');
      expect(trace).not.toHaveBeenCalled();
    } finally {
      trace.mockRestore();
    }
  });

  it('keeps an unexpected active-client abort at error level', () => {
    const output = sink();
    const lifecycle = new MatrixLifecycleLogger(output);
    const error = new DOMException('Aborted', 'AbortError');
    lifecycle.logger.error('Getting push rules failed', error);
    expect(output.error).toHaveBeenCalledWith('Getting push rules failed', error);
    expect(output.debug).not.toHaveBeenCalled();
  });

  it('retains intentional shutdown cancellation as a debug diagnostic in existing children', () => {
    const output = sink();
    const lifecycle = new MatrixLifecycleLogger(output);
    const child = lifecycle.logger.getChild('sync');
    const error = new DOMException('Aborted', 'AbortError');
    lifecycle.beginShutdown();
    child.error('Getting push rules failed', error);
    expect(output.debug).toHaveBeenCalledWith('Getting push rules failed', error);
    expect(output.error).not.toHaveBeenCalled();
  });

  it('continues reporting real failures after shutdown begins', () => {
    const output = sink();
    const lifecycle = new MatrixLifecycleLogger(output);
    const error = new Error('Server rejected logout');
    lifecycle.beginShutdown();
    lifecycle.logger.error('Logout failed', error);
    expect(output.error).toHaveBeenCalledWith('Logout failed', error);
    expect(output.debug).not.toHaveBeenCalled();
  });

  it('退出时中止的加密外发请求，SDK 拼成一句话记的，也只记成调试信息', () => {
    // 真实网页登录验收偶发红在这里：退出登录时 stopClient 中止了还在发的 keys/query。
    const output = sink();
    const lifecycle = new MatrixLifecycleLogger(output);
    const crypto = lifecycle.logger.getChild('[Rust crypto]');
    const message =
      'Failed to process outgoing request 1: AbortError: signal is aborted without reason';
    crypto.error(message);
    expect(output.error).toHaveBeenCalledWith(message);
    lifecycle.beginShutdown();
    crypto.error(message);
    expect(output.debug).toHaveBeenCalledWith(message);
    expect(output.error).toHaveBeenCalledOnce();
  });

  it('does not change another client or a string that merely mentions AbortError', () => {
    const output = sink();
    const first = new MatrixLifecycleLogger(output);
    const other = new MatrixLifecycleLogger(output);
    first.beginShutdown();
    first.logger.error('AbortError text is not a cancellation object');
    const error = new DOMException('Unexpected', 'AbortError');
    other.logger.error('Other client failed', error);
    expect(output.error).toHaveBeenCalledTimes(2);
    expect(output.debug).not.toHaveBeenCalled();
  });
});
