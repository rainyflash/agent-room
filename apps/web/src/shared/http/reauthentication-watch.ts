const REAUTHENTICATION_REQUIRED = 'authentication.reauthentication_required';

/**
 * 控制面说这个操作要最近登录过（401 `authentication.reauthentication_required`）时通知会话。
 * 会话只在读的那一刻知道“最近登录过”，各处的确认框照它显示：服务器拒过一次以后还当登录过，
 * 关掉再打开又让人白填一遍、再被拒一次（agent-room-ops#34）。这里看所有发往控制面的请求，
 * 以后新加要最近登录的操作也不用自己接。读的是回答的副本，调用方照常读自己的那份。
 */
export class ReauthenticationWatch {
  readonly #listeners = new Set<() => void>();
  readonly fetch: typeof globalThis.fetch;

  constructor(fetchImplementation: typeof globalThis.fetch) {
    this.fetch = async (input, init) => {
      const response = await fetchImplementation(input, init);
      if (response.status === 401) void this.#inspect(response.clone());
      return response;
    };
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  async #inspect(response: Response): Promise<void> {
    let body: unknown;
    try {
      body = await response.json();
    } catch {
      // 不是 JSON 的 401 不是要找的那种回答。
      return;
    }
    if (typeof body !== 'object' || body === null || !('code' in body)) return;
    if (body.code !== REAUTHENTICATION_REQUIRED) return;
    for (const listener of this.#listeners) listener();
  }
}
