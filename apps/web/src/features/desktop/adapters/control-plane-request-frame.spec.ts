import { describe, expect, it } from 'vitest';

import {
  decodeControlPlaneResponse,
  encodeControlPlaneRequest,
} from '@/features/desktop/adapters/control-plane-request-frame';

/** 按 `control_plane_proxy.rs` 的写法拼一段原生层的回答。 */
function nativeFrame(head: unknown, body: Uint8Array = new Uint8Array()): Uint8Array {
  const encoded = new TextEncoder().encode(JSON.stringify(head));
  const frame = new Uint8Array(4 + encoded.byteLength + body.byteLength);
  new DataView(frame.buffer).setUint32(0, encoded.byteLength);
  frame.set(encoded, 4);
  frame.set(body, 4 + encoded.byteLength);
  return frame;
}

describe('交给原生层代发的控制面请求', () => {
  it('请求编成“长度 + JSON 头 + 正文”，与原生层的解法一致', () => {
    const body = new TextEncoder().encode('{"a":1}');
    const frame = encodeControlPlaneRequest({
      body,
      headers: [['content-type', 'application/json']],
      method: 'POST',
      path: 'v1/things?cursor=x',
    });

    const length = new DataView(frame.buffer).getUint32(0);
    expect(JSON.parse(new TextDecoder().decode(frame.subarray(4, 4 + length)))).toEqual({
      headers: [['content-type', 'application/json']],
      method: 'POST',
      path: 'v1/things?cursor=x',
    });
    expect(new TextDecoder().decode(frame.subarray(4 + length))).toBe('{"a":1}');
  });

  it('收得下自定义协议的 ArrayBuffer，也收得下退回消息通道时的数字数组', () => {
    const frame = nativeFrame(
      { headers: [['content-type', 'application/json']], status: 201 },
      new TextEncoder().encode('{"ok":true}'),
    );
    const expected = {
      body: new TextEncoder().encode('{"ok":true}'),
      headers: [['content-type', 'application/json']],
      status: 201,
    };

    expect(decodeControlPlaneResponse(frame.buffer)).toEqual(expected);
    expect(decodeControlPlaneResponse(frame)).toEqual(expected);
    expect(decodeControlPlaneResponse(Array.from(frame))).toEqual(expected);
  });

  it('格式不对的回答一律不认', () => {
    const tooLong = nativeFrame({ headers: [], status: 200 });
    new DataView(tooLong.buffer).setUint32(0, 9_999);

    for (const invalid of [
      null,
      'text',
      { status: 200 },
      [256],
      new Uint8Array([0, 0]),
      tooLong,
      new Uint8Array([0, 0, 0, 1, 0x7b]),
      nativeFrame({ headers: [], status: 101 }),
      nativeFrame({ headers: [], status: 200, url: 'https://evil.example' }),
      nativeFrame({ headers: [['only-name']], status: 200 }),
    ]) {
      expect(decodeControlPlaneResponse(invalid)).toBeNull();
    }
  });
});
