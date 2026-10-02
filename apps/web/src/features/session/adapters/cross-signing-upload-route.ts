import {
  CONTROL_PLANE_UPLOAD,
  CrossSigningUploadFailure,
  type EncryptionKeyEscrow,
} from './matrix-device-signing';

const UPLOAD_PATH = '/_matrix/client/v3/keys/device_signing/upload';

/**
 * 包一层 fetch：设备重建签名身份时（ADR 0011），matrix-js-sdk 上传新签名公钥的请求带着我们自己的
 * 认证标记（`CONTROL_PLANE_UPLOAD`），这一个请求改送控制面，由它以应用服务的身份代传，人不用做
 * 任何交互认证。别的请求原样放行。
 *
 * 控制面的回答换成 Matrix 的样子交还给 matrix-js-sdk：成功是 200；控制面拒绝（公钥不对、太勤）
 * 是 400，它不会重试；控制面暂时不可用是 503，它会退避重试几次。
 */
export function routeCrossSigningUpload(
  fetchFn: typeof fetch,
  escrow: Pick<EncryptionKeyEscrow, 'replaceCrossSigningKeys'>,
): typeof fetch {
  return async (input, init) => {
    const keys = controlPlaneUpload(input, init);
    if (keys === null) return await fetchFn(input, init);
    try {
      await escrow.replaceCrossSigningKeys(keys);
      return matrixResponse(200, {});
    } catch (error) {
      const retryable = !(error instanceof CrossSigningUploadFailure) || error.retryable;
      return matrixResponse(retryable ? 503 : 400, {
        errcode: retryable ? 'M_UNKNOWN' : 'M_FORBIDDEN',
        error: 'Agent Room 控制面没能代传新的签名公钥。',
      });
    }
  };
}

/** 是带着我们认证标记的签名公钥上传，就取出公钥（去掉认证）；否则是 `null`。 */
function controlPlaneUpload(
  input: Parameters<typeof fetch>[0],
  init: Parameters<typeof fetch>[1],
): Record<string, unknown> | null {
  if (init?.method?.toUpperCase() !== 'POST' || typeof init.body !== 'string') return null;
  const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  if (!new URL(url, window.location.href).pathname.endsWith(UPLOAD_PATH)) return null;
  let body: unknown;
  try {
    body = JSON.parse(init.body);
  } catch {
    return null;
  }
  if (!isRecord(body) || !isRecord(body.auth) || body.auth.type !== CONTROL_PLANE_UPLOAD) {
    return null;
  }
  return Object.fromEntries(Object.entries(body).filter(([name]) => name !== 'auth'));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function matrixResponse(status: number, body: Record<string, unknown>): Response {
  return new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
    status,
  });
}
