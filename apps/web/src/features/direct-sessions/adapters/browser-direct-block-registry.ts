import { z } from 'zod';

import type { DirectBlockRegistry } from '@/features/direct-sessions/domain/direct-session';

const STORAGE_KEY = 'agent-room.direct-blocks.v1';
const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const storedBlocksSchema = z.array(uuidV7Schema).max(5_000);

/**
 * 维护先于网络副作用生效的本地屏蔽集合。
 *
 * 存储不可用时退化为当前进程内存，不会因此解除已经选择的屏蔽。
 */
export class BrowserDirectBlockRegistry implements DirectBlockRegistry {
  readonly #blocked: Set<string>;
  readonly #storage: Pick<Storage, 'getItem' | 'setItem'>;

  constructor(storage: Pick<Storage, 'getItem' | 'setItem'> = window.localStorage) {
    this.#storage = storage;
    this.#blocked = new Set(readStoredBlocks(storage));
  }

  has(agentId: string): boolean {
    return this.#blocked.has(agentId);
  }

  set(agentId: string, blocked: boolean): void {
    if (!uuidV7Schema.safeParse(agentId).success) {
      return;
    }
    if (blocked) {
      this.#blocked.add(agentId);
    } else {
      this.#blocked.delete(agentId);
    }
    this.#persist();
  }

  #persist(): void {
    try {
      this.#storage.setItem(STORAGE_KEY, JSON.stringify([...this.#blocked].toSorted()));
    } catch {
      // 内存集合是完整回退边界；存储恢复前仍保持当前进程的屏蔽状态。
    }
  }
}

function readStoredBlocks(storage: Pick<Storage, 'getItem'>): readonly string[] {
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (raw === null) {
      return [];
    }
    const parsedJson: unknown = JSON.parse(raw);
    const parsed = storedBlocksSchema.safeParse(parsedJson);
    return parsed.success ? parsed.data : [];
  } catch {
    return [];
  }
}
