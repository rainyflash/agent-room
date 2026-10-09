import type { Logger } from 'matrix-js-sdk/lib/logger.js';
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

describe('Matrix shutdown logging', () => {
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
