import { afterEach, describe, expect, it, vi } from 'vitest';

import type {
  DesktopControlPlaneRequest,
  DesktopControlPlaneResponse,
} from '@/features/desktop/domain/desktop-runtime';
import { ControlPlaneSharePreviewClient } from '@/features/room-share/adapters/control-plane-share-preview-client';
import { desktopControlPlaneFetch } from '@/features/session/adapters/desktop-control-plane-fetch';
import { ok } from '@/shared/result';

/** PNG 的文件头加几个字节，够认出是同一份。 */
const pngBytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

function png(headers: Record<string, string> = { 'content-type': 'image/png' }) {
  return new Response(pngBytes, { headers, status: 200 });
}

function client(fetch: typeof globalThis.fetch, timeoutMs?: number) {
  return new ControlPlaneSharePreviewClient({
    baseUrl: 'https://app.agent-room.test/_agent-room/api',
    fetch,
    ...(timeoutMs === undefined ? {} : { timeoutMs }),
  });
}

async function bytes(image: Blob | null): Promise<number[] | null> {
  return image === null ? null : [...new Uint8Array(await image.arrayBuffer())];
}

afterEach(() => {
  vi.useRealTimers();
});

describe('这间房自己的预览图', () => {
  it('经注入的 fetch 去要 /r/<分享码>/preview.png：不带 v、不带登录、不跟跳转、拿 ETag 去问', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(png());

    const image = await client(fetch).roomImage('aB3xYz', new AbortController().signal);

    expect(await bytes(image)).toEqual([...pngBytes]);
    expect(image?.type).toBe('image/png');
    expect(fetch).toHaveBeenCalledWith(
      new URL('https://app.agent-room.test/_agent-room/api/r/aB3xYz/preview.png'),
      expect.objectContaining({
        cache: 'no-cache',
        credentials: 'omit',
        headers: { Accept: 'image/png' },
        method: 'GET',
        redirect: 'manual',
      }),
    );
  });

  it('不是一张 PNG 的一律当作没有：404、服务器忙着跳去通用图、不是图、空的、连不上', async () => {
    const answers: (() => Promise<Response>)[] = [
      () => Promise.resolve(new Response('<!doctype html>', { status: 404 })),
      // 浏览器不跟跳转时拿到的是 opaqueredirect，状态是 0；桌面端原生层原样交回 302。
      () =>
        Promise.resolve(
          new Response(null, {
            headers: { location: 'https://app.agent-room.test/room-share-preview-en.png' },
            status: 302,
          }),
        ),
      () => Promise.resolve(Response.json({ code: 'oops' })),
      () =>
        Promise.resolve(
          new Response(new Uint8Array(), { headers: { 'content-type': 'image/png' } }),
        ),
      () => Promise.reject(new TypeError('offline')),
    ];
    for (const next of answers) {
      const fetch = vi.fn<typeof globalThis.fetch>().mockImplementation(next);
      await expect(
        client(fetch).roomImage('aB3xYz', new AbortController().signal),
      ).resolves.toBeNull();
    }
  });

  it('类型头带参数、大小写不一也认；没写类型的 Blob 补成 image/png', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(png({ 'content-type': 'Image/PNG; charset=binary' }));

    const image = await client(fetch).roomImage('aB3xYz', new AbortController().signal);

    expect(image?.type).toBe('image/png');
    expect(await bytes(image)).toEqual([...pngBytes]);
  });

  it('不像分享码的不去问服务器', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>();

    for (const code of ['', 'aB3xY', 'aB3xYz7', '../x/y', 'aB3%2F']) {
      await expect(client(fetch).roomImage(code, new AbortController().signal)).resolves.toBeNull();
    }
    expect(fetch).not.toHaveBeenCalled();
  });

  it('对话框关了就停下请求；一开始就停了的不发', async () => {
    const before = new AbortController();
    before.abort();
    const idle = vi.fn<typeof globalThis.fetch>();
    await expect(client(idle).roomImage('aB3xYz', before.signal)).resolves.toBeNull();
    expect(idle).not.toHaveBeenCalled();

    const closing = new AbortController();
    const fetch = vi.fn<typeof globalThis.fetch>().mockImplementation(
      (_input, init) =>
        new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        }),
    );
    const pending = client(fetch).roomImage('aB3xYz', closing.signal);
    closing.abort();
    await expect(pending).resolves.toBeNull();
  });

  it('服务器老不回就算了，放通用图', async () => {
    vi.useFakeTimers();
    let signal: AbortSignal | undefined;
    const fetch = vi.fn<typeof globalThis.fetch>().mockImplementation(
      (_input, init) =>
        new Promise((_resolve, reject) => {
          signal = init?.signal ?? undefined;
          signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        }),
    );
    const pending = client(fetch, 8_000).roomImage('aB3xYz', new AbortController().signal);
    await vi.advanceTimersByTimeAsync(8_000);
    await expect(pending).resolves.toBeNull();
    expect(signal?.aborted).toBe(true);
  });

  it('桌面端经原生层代发：路径相对控制面根地址，图的字节原样拿回；302 当作没有', async () => {
    const requests: DesktopControlPlaneRequest[] = [];
    let response: DesktopControlPlaneResponse = {
      body: pngBytes.slice(),
      headers: [
        ['content-type', 'image/png'],
        ['etag', '"0123456789abcdef"'],
      ],
      status: 200,
    };
    const native = {
      sendControlPlaneRequest: vi.fn((request: DesktopControlPlaneRequest) => {
        requests.push(request);
        return Promise.resolve(ok(response));
      }),
    };
    const fallback = vi.fn<typeof globalThis.fetch>();
    const previews = new ControlPlaneSharePreviewClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: desktopControlPlaneFetch('https://api.agent-room.test', native, fallback),
    });

    const image = await previews.roomImage('aB3xYz', new AbortController().signal);

    expect(await bytes(image)).toEqual([...pngBytes]);
    expect(fallback).not.toHaveBeenCalled();
    expect(requests[0]).toMatchObject({ method: 'GET', path: 'r/aB3xYz/preview.png' });

    response = {
      body: new Uint8Array(),
      headers: [['location', 'https://app.agent-room.test/room-share-preview-en.png']],
      status: 302,
    };
    await expect(previews.roomImage('aB3xYz', new AbortController().signal)).resolves.toBeNull();
  });
});
