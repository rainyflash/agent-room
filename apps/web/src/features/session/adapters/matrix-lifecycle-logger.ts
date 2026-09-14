import type { Logger } from 'matrix-js-sdk/lib/logger.js';

/** SDK sync startup logs an intentional HTTP abort as an error after stopClient. */
export class MatrixLifecycleLogger {
  #stopping = false;
  readonly logger: Logger;

  constructor(sink: Logger = consoleLogger('')) {
    this.logger = this.#scope(sink);
  }

  beginShutdown(): void {
    this.#stopping = true;
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
    trace: (...details: unknown[]) => {
      console.trace(...scoped(details));
    },
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

function isAbortError(value: unknown): boolean {
  return (
    typeof value === 'object' && value !== null && 'name' in value && value.name === 'AbortError'
  );
}
