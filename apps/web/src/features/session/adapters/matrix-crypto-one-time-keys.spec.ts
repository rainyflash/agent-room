import { afterEach, describe, expect, it } from 'vitest';
import type { MatrixClient } from 'matrix-js-sdk';

// 用真实的 matrix-js-sdk 同步循环和 Rust 加密（WASM）跑一遍：收到 to-device 消息时，
// 客户端不能以为服务器上的一次性密钥已经用光。matrix-js-sdk 42.2.0 把 to-device 消息和
// 密钥数量分两次交给加密层，前一次没带数量，crypto-wasm 18.5.0 起把“没带”当成 0，于是每批
// to-device 消息都多传 50 个一次性密钥。服务器按上传先后发放，客户端最多留 5000 个私钥，
// 积压超过 5000 个后，Agent 领到的就是客户端早已丢掉的密钥，Olm 通道建不起来，房间消息全都解不开。

const USER_ID = '@human:agent-room.test';
const DEVICE_ID = 'HUMANDEVICE';
const BASE_URL = 'https://matrix.agent-room.test';

type Deferred = { readonly promise: Promise<void>; readonly resolve: () => void };

function deferred(): Deferred {
  let resolve = (): void => undefined;
  const promise = new Promise<void>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

class FakeHomeserver {
  readonly uploadedOneTimeKeys: string[] = [];
  readonly #firstUploadAnswered = deferred();
  readonly #secondSyncProcessed = deferred();
  #syncs = 0;

  get secondSyncProcessed(): Promise<void> {
    return this.#secondSyncProcessed.promise;
  }

  readonly fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = new URL(input instanceof Request ? input.url : input.toString());
    const method = (init?.method ?? 'GET').toUpperCase();
    const path = url.pathname;
    if (path === '/_matrix/client/versions') {
      return json({ versions: ['v1.1', 'v1.5', 'v1.11', 'v1.12'], unstable_features: {} });
    }
    if (path === '/_matrix/client/v3/sync') return this.#sync(init?.signal ?? undefined);
    if (path === '/_matrix/client/v3/keys/upload') return this.#upload(init?.body);
    if (path === '/_matrix/client/v3/keys/query') {
      return json({ device_keys: { [USER_ID]: {} }, failures: {} });
    }
    if (path === '/_matrix/client/v3/pushrules/') {
      return json({ global: { content: [], override: [], room: [], sender: [], underride: [] } });
    }
    if (path.endsWith('/filter') && method === 'POST') return json({ filter_id: 'filter' });
    if (method === 'GET') return json({ errcode: 'M_NOT_FOUND', error: 'Not found' }, 404);
    return json({});
  };

  async #sync(signal: AbortSignal | undefined): Promise<Response> {
    this.#syncs += 1;
    switch (this.#syncs) {
      case 1:
        return json({
          next_batch: 's1',
          device_one_time_keys_count: { signed_curve25519: 0 },
          device_unused_fallback_key_types: [],
        });
      case 2:
        // 等新账户把自带的 50 个一次性密钥传完、服务器回了数量，再送一批 to-device 消息。
        await this.#firstUploadAnswered.promise;
        return json({
          next_batch: 's2',
          to_device: {
            events: [
              {
                content: { body: 'room key share placeholder' },
                sender: '@agent:agent-room.test',
                type: 'io.github.rainyflash.agentroom.test.ping',
              },
            ],
          },
          device_one_time_keys_count: { signed_curve25519: 50 },
          device_unused_fallback_key_types: ['signed_curve25519'],
        });
      default:
        // 第三次同步请求说明第二批已经处理完；之后一直挂起，直到客户端停止。
        this.#secondSyncProcessed.resolve();
        return new Promise<Response>((_resolve, reject) => {
          signal?.addEventListener('abort', () => {
            reject(new DOMException('Aborted', 'AbortError'));
          });
        });
    }
  }

  #upload(body: BodyInit | null | undefined): Response {
    const request = JSON.parse(typeof body === 'string' ? body : '{}') as {
      one_time_keys?: Record<string, unknown>;
    };
    this.uploadedOneTimeKeys.push(...Object.keys(request.one_time_keys ?? {}));
    queueMicrotask(() => {
      this.#firstUploadAnswered.resolve();
    });
    return json({ one_time_key_counts: { signed_curve25519: this.uploadedOneTimeKeys.length } });
  }
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    headers: { 'content-type': 'application/json' },
    status,
  });
}

const silentLogger = {
  debug: () => undefined,
  error: () => undefined,
  getChild: () => silentLogger,
  info: () => undefined,
  trace: () => undefined,
  warn: () => undefined,
};

type PendingRequest = { readonly type: number; readonly body: string };

let client: MatrixClient | undefined;

afterEach(() => {
  client?.stopClient();
  client = undefined;
});

describe('Matrix 加密同步', () => {
  it('收到 to-device 消息时不会把服务器上的一次性密钥数量当成 0 而多传一批', async () => {
    const sdk = await import('matrix-js-sdk');
    const server = new FakeHomeserver();
    client = sdk.createClient({
      accessToken: 'token',
      baseUrl: BASE_URL,
      deviceId: DEVICE_ID,
      fetchFn: server.fetch,
      logger: silentLogger,
      userId: USER_ID,
    });
    await client.initRustCrypto({ useIndexedDB: false });
    await client.startClient({ initialSyncLimit: 1 });
    await server.secondSyncProcessed;

    // 已经发出去的，加上加密层还攒着没发的，一共只能是新账户自带的那 50 个。
    const olmMachine = (
      client.getCrypto() as unknown as {
        olmMachine: { outgoingRequests(): Promise<PendingRequest[]> };
      }
    ).olmMachine;
    const pending = await olmMachine.outgoingRequests();
    const pendingOneTimeKeys = pending.flatMap((request) => {
      const body = JSON.parse(request.body) as { one_time_keys?: Record<string, unknown> };
      return Object.keys(body.one_time_keys ?? {});
    });
    expect(new Set([...server.uploadedOneTimeKeys, ...pendingOneTimeKeys]).size).toBe(50);
  }, 30_000);
});
