import { z } from 'zod';

import { BrowserMatrixSessionVault } from './browser-matrix-session-vault';
import {
  storedMatrixSessionSchema,
  type MatrixSessionVault,
  type StoredMatrixSession,
} from '../domain/matrix-session-vault';
import type { SessionFailure } from '../domain/session';
import { err, ok, type Result } from '@/shared/result';

const recordSchema = z
  .object({
    revision: z.uuid(),
    session: storedMatrixSessionSchema.nullable(),
  })
  .strict();
type SessionRecord = z.infer<typeof recordSchema>;

/** 持久化浏览器设备；注销墓碑和版本检查防止其他窗口把旧凭据写回。 */
export class IndexedDbMatrixSessionVault implements MatrixSessionVault {
  readonly #key: string;
  readonly #legacy: BrowserMatrixSessionVault;
  #revision: string | null | undefined;

  constructor(
    baseUrl: string,
    private readonly databaseFactory: IDBFactory | undefined,
    legacyStorage: Storage,
  ) {
    this.#key = new URL(baseUrl).href.replace(/\/+$/u, '');
    this.#legacy = new BrowserMatrixSessionVault(legacyStorage);
  }

  async load(): Promise<Result<StoredMatrixSession | null, SessionFailure>> {
    const legacy = await this.#legacy.load();
    const result = await this.transact((value) => {
      const existing = decodeRecord(value);
      if (existing !== null) return { value: existing };
      if (!legacy.ok) throw new VaultFailure(legacy.error);
      if (legacy.value === null) return { value: null };
      const migrated = { revision: crypto.randomUUID(), session: legacy.value };
      return { value: migrated, write: migrated };
    });
    if (!result.ok) return result;
    this.#revision = result.value?.revision ?? null;
    if (result.value !== null) {
      const cleared = await this.#legacy.clear();
      if (!cleared.ok) return cleared;
    }
    return ok(result.value?.session ?? null);
  }

  async save(session: StoredMatrixSession): Promise<Result<void, SessionFailure>> {
    const parsed = storedMatrixSessionSchema.safeParse(session);
    if (!parsed.success) return err(vaultFailure('matrix.invalid_session', false));
    const result = await this.transact((value) => {
      const current = decodeRecord(value);
      if (this.#revision !== undefined && this.#revision !== (current?.revision ?? null)) {
        throw new VaultFailure(vaultFailure('matrix.session_superseded', true));
      }
      const saved = { revision: crypto.randomUUID(), session: parsed.data };
      return { value: saved, write: saved };
    });
    if (!result.ok) return result;
    this.#revision = result.value.revision;
    return this.#legacy.clear();
  }

  async clear(): Promise<Result<void, SessionFailure>> {
    const result = await this.transact(() => {
      const tombstone = { revision: crypto.randomUUID(), session: null };
      return { value: tombstone, write: tombstone };
    });
    if (!result.ok) return result;
    this.#revision = result.value.revision;
    return this.#legacy.clear();
  }

  private async transact<TValue>(
    update: (current: unknown) => { readonly value: TValue; readonly write?: SessionRecord },
  ): Promise<Result<TValue, SessionFailure>> {
    let database: IDBDatabase | undefined;
    try {
      database = await this.open();
      const transaction = database.transaction('matrix', 'readwrite');
      const store = transaction.objectStore('matrix');
      const value = await new Promise<TValue>((resolve, reject) => {
        let outcome: { readonly value: TValue } | undefined;
        let failure: unknown;
        transaction.onabort = () => {
          reject(
            failure instanceof Error
              ? failure
              : new Error('Session transaction aborted', { cause: failure ?? transaction.error }),
          );
        };
        transaction.oncomplete = () => {
          if (outcome === undefined) reject(new Error('Missing committed session result'));
          else resolve(outcome.value);
        };
        const request = store.get(this.#key);
        request.onsuccess = () => {
          try {
            const current: unknown = request.result;
            const next = update(current);
            if (next.write !== undefined) store.put(next.write, this.#key);
            outcome = next;
          } catch (error: unknown) {
            failure = error;
            transaction.abort();
          }
        };
      });
      return ok(value);
    } catch (error: unknown) {
      return err(
        error instanceof VaultFailure
          ? error.failure
          : vaultFailure('browser.session_storage_unavailable', true),
      );
    } finally {
      database?.close();
    }
  }

  private open(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      if (this.databaseFactory === undefined) {
        reject(new Error('IndexedDB is unavailable'));
        return;
      }
      const request = this.databaseFactory.open('agent-room.sessions.v1', 1);
      let blocked = false;
      request.onupgradeneeded = () => request.result.createObjectStore('matrix');
      request.onerror = () => {
        reject(new Error('Could not open session database', { cause: request.error }));
      };
      request.onblocked = () => {
        blocked = true;
        reject(new Error('Session database upgrade is blocked'));
      };
      request.onsuccess = () => {
        if (blocked) request.result.close();
        else {
          request.result.onversionchange = () => {
            request.result.close();
          };
          resolve(request.result);
        }
      };
    });
  }
}

function decodeRecord(value: unknown): SessionRecord | null {
  if (value === undefined) return null;
  const parsed = recordSchema.safeParse(value);
  if (!parsed.success)
    throw new VaultFailure(vaultFailure('browser.session_storage_corrupt', false));
  return parsed.data;
}

function vaultFailure(code: string, retryable: boolean): SessionFailure {
  return { boundary: 'browser', code, offline: false, retryable };
}

class VaultFailure extends Error {
  constructor(readonly failure: SessionFailure) {
    super(failure.code);
  }
}
