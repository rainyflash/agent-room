import type { CryptoCallbacks } from 'matrix-js-sdk/lib/crypto-api/index.js';

export class MatrixSecretStorageKeyCache {
  readonly #keys = new Map<string, Uint8Array<ArrayBuffer>>();

  readonly callbacks: CryptoCallbacks = {
    cacheSecretStorageKey: (keyId, _keyInfo, key) => {
      this.unlock(keyId, key);
    },
    getSecretStorageKey: ({ keys }) => Promise.resolve(this.#findRequestedKey(Object.keys(keys))),
  };

  clear(): void {
    for (const key of this.#keys.values()) {
      key.fill(0);
    }
    this.#keys.clear();
  }

  unlock(keyId: string, key: Uint8Array): void {
    const previous = this.#keys.get(keyId);
    previous?.fill(0);
    this.#keys.set(keyId, Uint8Array.from(key));
  }

  #findRequestedKey(keyIds: readonly string[]): [string, Uint8Array<ArrayBuffer>] | null {
    for (const keyId of keyIds) {
      const key = this.#keys.get(keyId);
      if (key !== undefined) {
        return [keyId, Uint8Array.from(key)];
      }
    }
    return null;
  }
}
