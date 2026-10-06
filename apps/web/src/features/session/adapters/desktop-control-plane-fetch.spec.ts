import { describe, expect, it, vi } from 'vitest';

import type {
  DesktopControlPlaneRequest,
  DesktopControlPlaneResponse,
} from '@/features/desktop/domain/desktop-runtime';
import { desktopControlPlaneFetch } from '@/features/session/adapters/desktop-control-plane-fetch';
import { err, ok } from '@/shared/result';

function answer(
  status: number,
  body = '',
  headers: [string, string][] = [],
): DesktopControlPlaneResponse {
  return { body: new TextEncoder().encode(body), headers, status };
}

function transport(response: DesktopControlPlaneResponse = answer(200, '{"ok":true}')) {
  const requests: DesktopControlPlaneRequest[] = [];
  const sendControlPlaneRequest = vi.fn((request: DesktopControlPlaneRequest) => {
    requests.push(request);
    return Promise.resolve(ok(response));
  });
  return { requests, sendControlPlaneRequest };
}

const text = (request: DesktopControlPlaneRequest | undefined) =>
  new TextDecoder().decode(request?.body);

describe('桌面端发往控制面的请求交给原生层代发', () => {
  it('控制面地址下的请求交给原生层：路径相对根地址，方法、请求头、正文原样带上', async () => {
    const native = transport(
      answer(201, '{"ok":true}', [
        ['content-type', 'application/json'],
        ['x-correlation-id', 'corr-1'],
      ]),
    );
    const fallback = vi.fn<typeof fetch>();
    const send = desktopControlPlaneFetch('https://api.example.test', native, fallback);

    const response = await send('https://api.example.test/v1/things?cursor=a%2Fb', {
      body: '{"a":1}',
      cache: 'no-store',
      credentials: 'include',
      headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
      method: 'post',
    });

    expect(fallback).not.toHaveBeenCalled();
    expect(native.requests).toHaveLength(1);
    expect(native.requests[0]).toMatchObject({
      headers: [
        ['accept', 'application/json'],
        ['content-type', 'application/json'],
      ],
      method: 'POST',
      path: 'v1/things?cursor=a%2Fb',
    });
    expect(text(native.requests[0])).toBe('{"a":1}');
    expect(response.status).toBe(201);
    expect(response.headers.get('x-correlation-id')).toBe('corr-1');
    await expect(response.json()).resolves.toEqual({ ok: true });
  });

  it('根地址带路径前缀时，路径相对前缀；前缀外和别的站点照常走 fetch', async () => {
    const native = transport();
    const fallback = vi.fn<typeof fetch>().mockResolvedValue(new Response('elsewhere'));
    const send = desktopControlPlaneFetch(
      'https://app.example.test/_agent-room/api',
      native,
      fallback,
    );

    await send(new URL('https://app.example.test/_agent-room/api/auth/session'));
    await send('https://app.example.test/_agent-room/apiary');
    await send('https://matrix.example.test/_matrix/client/versions');

    expect(native.requests.map(({ method, path }) => ({ method, path }))).toEqual([
      { method: 'GET', path: 'auth/session' },
    ]);
    expect(text(native.requests[0])).toBe('');
    expect(fallback).toHaveBeenCalledTimes(2);
  });

  it('收得下 Request 对象，表单正文自带的类型（含 multipart 边界）没写时补上', async () => {
    const native = transport();
    const send = desktopControlPlaneFetch('https://api.example.test/', native);

    await send(new Request('https://api.example.test/content', { body: 'raw', method: 'PUT' }));
    const form = new FormData();
    form.set('name', 'Ada');
    await send('https://api.example.test/form', { body: form, method: 'POST' });
    await send('https://api.example.test/search', {
      body: new URLSearchParams({ q: 'room' }),
      method: 'POST',
    });

    expect(native.requests[0]?.method).toBe('PUT');
    expect(text(native.requests[0])).toBe('raw');
    const multipart = new Map(native.requests[1]?.headers).get('content-type');
    expect(multipart).toMatch(/^multipart\/form-data; boundary=/u);
    expect(text(native.requests[1])).toContain('Ada');
    expect(new Map(native.requests[2]?.headers).get('content-type')).toBe(
      'application/x-www-form-urlencoded;charset=UTF-8',
    );
    expect(text(native.requests[2])).toBe('q=room');
  });

  it('204 和 HEAD 的回答没有正文', async () => {
    const send = desktopControlPlaneFetch('https://api.example.test', transport(answer(204)));
    const empty = await send('https://api.example.test/auth/logout', { method: 'POST' });
    expect(empty.status).toBe(204);
    expect(empty.body).toBeNull();

    const head = await desktopControlPlaneFetch(
      'https://api.example.test',
      transport(answer(200, 'ignored')),
    )('https://api.example.test/health/ready', { method: 'HEAD' });
    expect(head.body).toBeNull();
  });

  it('原生层发不出去时像 fetch 一样抛 TypeError', async () => {
    const send = desktopControlPlaneFetch('https://api.example.test', {
      sendControlPlaneRequest: () =>
        Promise.resolve(err({ code: 'desktop.control_plane.unavailable', retryable: true })),
    });

    await expect(send('https://api.example.test/agents')).rejects.toBeInstanceOf(TypeError);
  });

  it('取消在先就不发；发出后取消立刻放手，不等原生层回答', async () => {
    const native = transport();
    const send = desktopControlPlaneFetch('https://api.example.test', native);
    const cancelled = new AbortController();
    cancelled.abort(new DOMException('cleared', 'AbortError'));

    await expect(
      send('https://api.example.test/agents', { signal: cancelled.signal }),
    ).rejects.toMatchObject({ name: 'AbortError' });
    expect(native.sendControlPlaneRequest).not.toHaveBeenCalled();

    // 读正文时取消：还没交给原生层，就不再交。
    const reading = new AbortController();
    const whileReading = send('https://api.example.test/agents', {
      body: 'x',
      method: 'POST',
      signal: reading.signal,
    });
    reading.abort(new DOMException('cleared', 'AbortError'));
    await expect(whileReading).rejects.toMatchObject({ name: 'AbortError' });
    expect(native.sendControlPlaneRequest).not.toHaveBeenCalled();

    const { promise: reached, resolve: reach } = Promise.withResolvers<undefined>();
    const hanging = desktopControlPlaneFetch('https://api.example.test', {
      sendControlPlaneRequest: () => {
        reach(undefined);
        return new Promise(() => undefined);
      },
    });
    const controller = new AbortController();
    const pending = hanging('https://api.example.test/agents', { signal: controller.signal });
    await reached;
    controller.abort(new DOMException('cleared', 'AbortError'));
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  });
});
