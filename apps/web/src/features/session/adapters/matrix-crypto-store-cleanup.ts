/**
 * 退出登录时没删掉的本机加密库，记下来下次启动再删。
 *
 * Rust 加密模块停下以后，要等它留下的对象都释放（有时要等垃圾回收）才关掉 IndexedDB 连接。
 * 在那之前删除加密库会一直被挡着，退出就会一直停在“正在关闭活跃会话”。设备和令牌那时已在
 * 服务器上撤销，这台设备也不会再被恢复，所以退出不必等它：删除请求会在连接关掉后自己完成，
 * 页面要是先关了，下次启动再删一次。
 */
const CLEANUP_KEY = 'agent-room.matrix-crypto-cleanup.v1';
const DATABASE_SUFFIXES = ['::matrix-sdk-crypto', '::matrix-sdk-crypto-meta'] as const;
const PREFIX_PATTERN = /^agent-room-crypto-[0-9a-f]{8}$/u;
const MAX_REMEMBERED = 20;

export class MatrixCryptoStoreCleanup {
  readonly #storage: Storage | undefined;
  readonly #indexedDB: IDBFactory | undefined;

  constructor(storage: Storage | undefined, indexedDB: IDBFactory | undefined) {
    this.#storage = storage;
    this.#indexedDB = indexedDB;
  }

  /** 这台设备的加密库还没删掉：下次启动时再删。 */
  defer(prefix: string): void {
    if (!PREFIX_PATTERN.test(prefix)) return;
    const pending = this.#read();
    if (!pending.includes(prefix)) this.#write([...pending, prefix].slice(-MAX_REMEMBERED));
  }

  /** 已经删掉了，不用再记着。 */
  done(prefix: string): void {
    const pending = this.#read();
    if (pending.includes(prefix)) this.#write(pending.filter((item) => item !== prefix));
  }

  /** 启动时删掉上次没删成的加密库。仍被占用或出错就留到下次。 */
  retry(): void {
    const factory = this.#indexedDB;
    if (factory === undefined) return;
    for (const prefix of this.#read()) {
      void deleteCryptoDatabases(factory, prefix).then((deleted) => {
        if (deleted) this.done(prefix);
      });
    }
  }

  #read(): string[] {
    try {
      const value: unknown = JSON.parse(this.#storage?.getItem(CLEANUP_KEY) ?? '[]');
      return Array.isArray(value)
        ? value.filter(
            (item): item is string => typeof item === 'string' && PREFIX_PATTERN.test(item),
          )
        : [];
    } catch {
      return [];
    }
  }

  #write(prefixes: readonly string[]): void {
    try {
      if (prefixes.length === 0) this.#storage?.removeItem(CLEANUP_KEY);
      else this.#storage?.setItem(CLEANUP_KEY, JSON.stringify(prefixes));
    } catch {
      // 记不下来也只是留下一个已撤销设备的加密库，不影响退出。
    }
  }
}

async function deleteCryptoDatabases(factory: IDBFactory, prefix: string): Promise<boolean> {
  for (const suffix of DATABASE_SUFFIXES) {
    const deleted = await new Promise<boolean>((resolve) => {
      try {
        const request = factory.deleteDatabase(`${prefix}${suffix}`);
        request.onsuccess = () => {
          resolve(true);
        };
        request.onerror = () => {
          resolve(false);
        };
        request.onblocked = () => {
          resolve(false);
        };
      } catch {
        resolve(false);
      }
    });
    if (!deleted) return false;
  }
  return true;
}
