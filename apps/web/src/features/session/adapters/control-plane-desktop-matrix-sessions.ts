import type { SessionFailure } from '@/features/session/domain/session';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok, type Result } from '@/shared/result';

/** 桌面端拿聊天登录：控制面替人登录 Matrix，回答和 Matrix 登录接口同一个形状。 */
export type DesktopMatrixSessions = {
  issue(): Promise<Result<unknown, SessionFailure>>;
};

/**
 * 桌面端的聊天登录由控制面以应用服务身份替人发（`specs/desktop-sign-in/design.md`），不再开一次系统
 * 浏览器、停在 Synapse 的确认页。请求经组合根注入的代发 fetch，原生层带上桌面登录。
 */
export class ControlPlaneDesktopMatrixSessions implements DesktopMatrixSessions {
  constructor(
    private readonly options: { readonly baseUrl: string; readonly fetch: typeof globalThis.fetch },
  ) {}

  async issue(): Promise<Result<unknown, SessionFailure>> {
    let response: Response;
    try {
      response = await this.options.fetch(
        controlPlaneEndpoint(this.options.baseUrl, '/auth/desktop/matrix-session'),
        {
          cache: 'no-store',
          credentials: 'include',
          headers: { Accept: 'application/json' },
          method: 'POST',
          signal: AbortSignal.timeout(20_000),
        },
      );
    } catch {
      return err({
        boundary: 'matrix',
        code: 'matrix.desktop_session_unreachable',
        offline: !globalThis.navigator.onLine,
        retryable: true,
      });
    }
    if (!response.ok) {
      const body = await errorBody(response);
      return err({
        boundary: 'matrix',
        code: body.code ?? `matrix.desktop_session_http_${String(response.status)}`,
        ...(body.correlationId === null ? {} : { correlationId: body.correlationId }),
        offline: false,
        // 太勤（429）、控制面或 Synapse 暂时不可用（5xx）：过一会儿能好。别的再试也一样。
        retryable: response.status === 429 || response.status >= 500,
      });
    }
    try {
      return ok(await response.json());
    } catch {
      return err({
        boundary: 'matrix',
        code: 'matrix.invalid_login_response',
        offline: false,
        retryable: false,
      });
    }
  }
}

async function errorBody(
  response: Response,
): Promise<{ readonly code: string | null; readonly correlationId: string | null }> {
  try {
    const body: unknown = await response.json();
    if (typeof body !== 'object' || body === null) return { code: null, correlationId: null };
    const code = 'code' in body && typeof body.code === 'string' ? body.code : null;
    const correlationId =
      'correlationId' in body && typeof body.correlationId === 'string' ? body.correlationId : null;
    return { code, correlationId };
  } catch {
    return { code: null, correlationId: null };
  }
}
