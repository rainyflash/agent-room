import { typingTimeoutMs, type TypingNotifier } from '../domain/typing';

/** 一直在打字时，离服务器那边到期还剩 10 秒就续一次。 */
const renewAfterMs = typingTimeoutMs - 10_000;
/** 这么久没再改输入框就当停了。 */
const idleAfterMs = 10_000;

/**
 * 输入框里的“正在输入”：开始打字时说一次，一直在打就隔一会儿续一次；
 * 停了 10 秒、清空、发出去或者离开房间就说停了。
 */
export class TypingNotice {
  readonly #notifier: TypingNotifier;
  readonly #roomId: string;
  readonly #now: () => number;
  /** 上次说“正在输入”是几点；没在打字时为空。 */
  #saidAt: number | null = null;
  #idle: ReturnType<typeof setTimeout> | null = null;

  constructor(notifier: TypingNotifier, roomId: string, now: () => number = Date.now) {
    this.#notifier = notifier;
    this.#roomId = roomId;
    this.#now = now;
  }

  /** 输入框的内容变了。 */
  changed(text: string): void {
    if (text.trim().length === 0) {
      this.stop();
      return;
    }
    const now = this.#now();
    if (this.#saidAt === null || now - this.#saidAt >= renewAfterMs) {
      this.#saidAt = now;
      this.#notifier.typing(this.#roomId, true);
    }
    if (this.#idle !== null) clearTimeout(this.#idle);
    this.#idle = setTimeout(() => {
      this.stop();
    }, idleAfterMs);
  }

  /** 不打了：发出去、清空、离开房间，或者停了一会儿。 */
  stop(): void {
    if (this.#idle !== null) {
      clearTimeout(this.#idle);
      this.#idle = null;
    }
    if (this.#saidAt === null) return;
    this.#saidAt = null;
    this.#notifier.typing(this.#roomId, false);
  }
}
