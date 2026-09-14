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
