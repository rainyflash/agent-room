import type { MatrixClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixClientRegistry } from './matrix-client-registry';

describe('MatrixClientRegistry', () => {
  it('仅在客户端引用真正变化时通知订阅者', () => {
    const registry = new MatrixClientRegistry();
    const listener = vi.fn();
    const client = {} as MatrixClient;
    const unsubscribe = registry.subscribe(listener);

    registry.replace(client);
    registry.replace(client);
    registry.refresh(client);
    registry.replace(null);
    registry.refresh(client);
    unsubscribe();
    registry.replace(client);

    expect(listener).toHaveBeenCalledTimes(3);
    expect(registry.current()).toBe(client);
  });
});
