import type { Logger } from 'matrix-js-sdk/lib/logger.js';

/** 退出以后多久之内还算退出留下的请求。SDK 接着往下走只隔几毫秒，这里留够余地。 */
const ABANDONED_REQUEST_WINDOW_MS = 10_000;

/** SDK sync startup logs an intentional HTTP abort as an error after stopClient. */
export class MatrixLifecycleLogger {
  #stopping = false;
  readonly #sink: Logger;
  readonly logger: Logger;

  constructor(sink: Logger = consoleLogger('')) {
    this.#sink = sink;
    this.logger = this.#scope(sink);
  }

  beginShutdown(): void {
    if (this.#stopping) return;
    this.#stopping = true;
    this.#quietAbandonedRequests();
  }

  /**
   * 退出正好落在启动同步的半路时，SDK 接着用已经中止的信号发第一个同步请求，再看到已经停了就退出，
   * 谁也不 await 那个请求（`SyncApi.sync`），页面上就多一条没人接的 AbortError。拿不到那个 Promise，
   * 所以退出后这一小段时间里，把没人接的 AbortError 记成调试信息。别的错照常报。
   */
  #quietAbandonedRequests(): void {
    if (typeof window === 'undefined') return;
    const onRejection = (event: PromiseRejectionEvent): void => {
      if (!isAbortError(event.reason)) return;
      event.preventDefault();
      this.#sink.debug('Abandoned Matrix request after shutdown', event.reason);
    };
    window.addEventListener('unhandledrejection', onRejection);
    window.setTimeout(() => {
      window.removeEventListener('unhandledrejection', onRejection);
    }, ABANDONED_REQUEST_WINDOW_MS);
  }

  #scope(sink: Logger): Logger {
    return {
      trace: sink.trace,
      debug: sink.debug,
      info: sink.info,
      warn: sink.warn,
      error: (...details: unknown[]) => {
        if (this.#stopping && details.some(isAbortError)) sink.debug(...details);
        else sink.error(...details);
      },
      getChild: (namespace: string) => this.#scope(sink.getChild(namespace)),
    };
  }
}

function consoleLogger(namespace: string): Logger {
  const scoped = (details: readonly unknown[]) =>
    namespace === '' ? details : [namespace, ...details];
  return {
    // Match the SDK's default DEBUG threshold; TRACE can contain key diagnostics.
    trace: () => undefined,
    debug: (...details: unknown[]) => {
      console.debug(...scoped(details));
    },
    info: (...details: unknown[]) => {
      console.info(...scoped(details));
    },
    warn: (...details: unknown[]) => {
      console.warn(...scoped(details));
    },
    error: (...details: unknown[]) => {
      console.error(...scoped(details));
    },
    getChild: (child: string) => consoleLogger(namespace === '' ? child : `${namespace} ${child}`),
  };
}

/**
 * Rust 加密的外发请求失败时，SDK 把错误拼进一句话里再记，没有错误对象可看
 * （`Failed to process outgoing request 1: AbortError: signal is aborted without reason`）。
 * 字符串只认这一句：只是提到 AbortError 的话不算取消。
 */
const OUTGOING_REQUEST_ABORT = /^Failed to process outgoing request \d+: AbortError: /u;

function isAbortError(value: unknown): boolean {
  if (typeof value === 'string') return OUTGOING_REQUEST_ABORT.test(value);
  return (
    typeof value === 'object' && value !== null && 'name' in value && value.name === 'AbortError'
  );
}
