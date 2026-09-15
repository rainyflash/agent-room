/** Binary drafts use IndexedDB; expanding bytes into Web Storage exhausts its small quota. */
export class BrowserBinaryStore {
  constructor(private readonly namespace: string) {}

  async read(key: string): Promise<unknown> {
    return this.run('readonly', (store) => store.get(`${this.namespace}:${key}`));
  }
  async write(key: string, value: unknown): Promise<void> {
    await this.run('readwrite', (store) => store.put(value, `${this.namespace}:${key}`));
  }
  async remove(key: string): Promise<void> {
    await this.run('readwrite', (store) => store.delete(`${this.namespace}:${key}`));
  }
  private async run<T>(
    mode: IDBTransactionMode,
    operation: (store: IDBObjectStore) => IDBRequest<T>,
  ): Promise<T> {
    const database = await new Promise<IDBDatabase>((resolve, reject) => {
      let settled = false;
      const timeout = setTimeout(() => {
        settled = true;
        reject(new Error('storage.timeout'));
      }, 10_000);
      const request = indexedDB.open('agent-room-binary-drafts-v1', 1);
      request.onupgradeneeded = () => {
        request.result.createObjectStore('entries');
      };
      request.onsuccess = () => {
        clearTimeout(timeout);
        if (settled) request.result.close();
        else {
          settled = true;
          resolve(request.result);
        }
      };
      request.onerror = () => {
        clearTimeout(timeout);
        settled = true;
        reject(new Error('storage.open_failed', { cause: request.error }));
      };
      request.onblocked = () => {
        clearTimeout(timeout);
        settled = true;
        reject(new Error('storage.blocked'));
      };
    });
    try {
      return await new Promise<T>((resolve, reject) => {
        const transaction = database.transaction('entries', mode);
        const request = operation(transaction.objectStore('entries'));
        transaction.oncomplete = () => {
          resolve(request.result);
        };
        transaction.onabort = () => {
          reject(new Error('storage.transaction_failed', { cause: transaction.error }));
        };
        transaction.onerror = () => {
          reject(new Error('storage.transaction_failed', { cause: transaction.error }));
        };
      });
    } finally {
      database.close();
    }
  }
}
