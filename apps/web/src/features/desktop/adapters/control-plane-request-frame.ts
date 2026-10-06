import { z } from 'zod';

import type {
  DesktopControlPlaneRequest,
  DesktopControlPlaneResponse,
} from '@/features/desktop/domain/desktop-runtime';

/**
 * 交给原生层代发的控制面请求，和它交回的回答，都是一段字节：开头 4 字节大端长度，
 * 接着这么长的 JSON 头，剩下的是正文。格式与 `control_plane_proxy.rs` 对应。
 */
const responseHeadSchema = z
  .object({
    status: z.number().int().min(200).max(599),
    headers: z.array(z.tuple([z.string(), z.string()])).max(256),
  })
  .strict();

export function encodeControlPlaneRequest(request: DesktopControlPlaneRequest): Uint8Array {
  const head = new TextEncoder().encode(
    JSON.stringify({ method: request.method, path: request.path, headers: request.headers }),
  );
  const frame = new Uint8Array(4 + head.byteLength + request.body.byteLength);
  new DataView(frame.buffer).setUint32(0, head.byteLength);
  frame.set(head, 4);
  frame.set(request.body, 4 + head.byteLength);
  return frame;
}

/** 原生层的回答。走自定义协议时是 `ArrayBuffer`，退回消息通道时是数字数组；不对就是 `null`。 */
export function decodeControlPlaneResponse(raw: unknown): DesktopControlPlaneResponse | null {
  const frame = bytes(raw);
  if (frame === null || frame.byteLength < 4) return null;
  const length = new DataView(frame.buffer, frame.byteOffset, frame.byteLength).getUint32(0);
  if (length > frame.byteLength - 4) return null;
  let head: unknown;
  try {
    head = JSON.parse(new TextDecoder().decode(frame.subarray(4, 4 + length)));
  } catch {
    return null;
  }
  const parsed = responseHeadSchema.safeParse(head);
  if (!parsed.success) return null;
  return {
    status: parsed.data.status,
    headers: parsed.data.headers,
    body: frame.slice(4 + length),
  };
}

function bytes(raw: unknown): Uint8Array | null {
  if (raw instanceof Uint8Array) return raw;
  if (raw instanceof ArrayBuffer) return new Uint8Array(raw);
  if (
    Array.isArray(raw) &&
    raw.every((value) => Number.isInteger(value) && value >= 0 && value <= 255)
  ) {
    return Uint8Array.from(raw as number[]);
  }
  return null;
}
