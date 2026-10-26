/**
 * 同一个浏览器里一次只让一个标签页走聊天登录（Synapse 的 SSO，agent-room-ops#17）。
 *
 * Synapse 把一次登录的凭据记在一个 Cookie 里，整个浏览器只有一个。两个标签页前后脚开始，后开始的会把
 * 前一个的盖掉，先开始的那个回来时就停在 Synapse 的报错页上。新注册的人最常碰上：在新标签页里点了验证
 * 邮件，那边登录完，原来那个标签页的登录页（Keycloak 盯着别的标签页）也自己刷新、回到网页，两边不到半秒
 * 都开始聊天登录。
 *
 * 开始前在 localStorage 记下哪个标签页、什么时候开始的，回来换好登录以后撤掉。别的标签页看到记号就等它
 * 走完，再用它存下的登录（同一个浏览器的聊天登录本来就是共用的一份）。记号过了 30 秒不算数，免得关掉的
 * 标签页一直挡着别人。
 */

const TURN_MS = 30_000;
const POLL_MS = 250;
const SETTLE_MS = 50;
const TAB_KEY = 'agent-room.tab-id.v1';

type Holder = { readonly at: number; readonly tab: string };

export type MatrixSignInTurnOptions = {
  readonly key: string;
  /** 各个标签页共用的（localStorage）；浏览器不让用时不排队。 */
  readonly shared: Pick<Storage, 'getItem' | 'removeItem' | 'setItem'> | undefined;
  /** 只属于这个标签页、跨过登录跳转还在的（sessionStorage），记着这个标签页是谁。 */
  readonly tab: Pick<Storage, 'getItem' | 'setItem'>;
  readonly now?: () => number;
  readonly pause?: (ms: number) => Promise<void>;
};

export class MatrixSignInTurn {
  readonly #key: string;
  readonly #shared: MatrixSignInTurnOptions['shared'];
  readonly #tab: MatrixSignInTurnOptions['tab'];
  readonly #now: () => number;
  readonly #pause: (ms: number) => Promise<void>;

  constructor({ key, now = Date.now, pause = sleep, shared, tab }: MatrixSignInTurnOptions) {
    this.#key = key;
    this.#shared = shared;
    this.#tab = tab;
    this.#now = now;
    this.#pause = pause;
  }

  /**
   * 轮到这个标签页：别的标签页正在走聊天登录就等它走完（最多到它开始后 30 秒），再记下自己开始了。
   * 回答等没等过别人：等过的话，多半已经有一份刚存下的登录可以直接用。
   */
  async take(): Promise<{ readonly waited: boolean }> {
    const tab = this.#tabId();
    if (tab === null) return { waited: false };
    let waited = false;
    for (;;) {
      const holder = this.#read();
      if (holder !== null && holder.tab !== tab && this.#now() - holder.at < TURN_MS) {
        waited = true;
        await this.#pause(POLL_MS);
        continue;
      }
      if (!this.#write({ at: this.#now(), tab })) return { waited };
      // 两个标签页同时记下时后写的算数：等一下再看还是不是自己。
      await this.#pause(SETTLE_MS);
      if (this.#read()?.tab === tab) return { waited };
      waited = true;
    }
  }

  /** 这个标签页的聊天登录走完了（成没成都算）：撤掉自己的记号，别的标签页不用再等。 */
  release(): void {
    const tab = this.#tabId();
    if (tab === null || this.#read()?.tab !== tab) return;
    try {
      this.#shared?.removeItem(this.#key);
    } catch {
      // 撤不掉也就是别人多等一会儿，30 秒后记号不算数。
    }
  }

  #tabId(): string | null {
    if (this.#shared === undefined) return null;
    try {
      const existing = this.#tab.getItem(TAB_KEY);
      if (existing !== null) return existing;
      const created = crypto.randomUUID();
      this.#tab.setItem(TAB_KEY, created);
      return created;
    } catch {
      return null;
    }
  }

  #read(): Holder | null {
    try {
      const raw = this.#shared?.getItem(this.#key) ?? null;
      if (raw === null) return null;
      const value: unknown = JSON.parse(raw);
      return isHolder(value) ? value : null;
    } catch {
      return null;
    }
  }

  #write(holder: Holder): boolean {
    try {
      this.#shared?.setItem(this.#key, JSON.stringify(holder));
      return true;
    } catch {
      return false;
    }
  }
}

function isHolder(value: unknown): value is Holder {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { at?: unknown }).at === 'number' &&
    typeof (value as { tab?: unknown }).tab === 'string'
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}
