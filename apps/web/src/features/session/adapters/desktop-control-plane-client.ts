import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import type { ReadinessGateway } from '@/features/health/domain/readiness';
import type {
  AuthenticationIntent,
  AuthenticationStartOutcome,
  ControlPlaneGateway,
  SessionFailure,
  WebSession,
} from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';

import { ControlPlaneClient } from './control-plane-client';

export type DesktopControlPlaneClientOptions = {
  readonly controlPlane: ControlPlaneClient;
  readonly navigate?: (path: string) => void;
  readonly runtime: DesktopRuntimeGateway;
};

/**
 * 只负责把云端用户会话协议适配到桌面安全边界；Bridge/Agent 凭据不进入此类。
 */
export class DesktopControlPlaneClient implements ControlPlaneGateway, ReadinessGateway {
  readonly #controlPlane: ControlPlaneClient;
  readonly #navigate: (path: string) => void;
  readonly #runtime: DesktopRuntimeGateway;

  constructor({
    controlPlane,
    navigate = (path) => {
      window.location.assign(path);
    },
    runtime,
  }: DesktopControlPlaneClientOptions) {
    this.#controlPlane = controlPlane;
    this.#navigate = navigate;
    this.#runtime = runtime;
  }

  async beginAuthentication(
    returnPath: string,
    intent: AuthenticationIntent = 'sign-in',
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>> {
    const result = await this.#runtime.beginHumanAuthentication(returnPath, intent);
    if (!result.ok) {
      return err(runtimeFailure(result.error));
    }
    try {
      // 重新加载确保所有请求都在 HttpOnly 桌面 Cookie 注入后启动。
      this.#navigate(result.value.returnPath);
      return ok({ kind: 'session-established' });
    } catch {
      return err({
        boundary: 'browser',
        code: 'browser.authentication_navigation_failed',
        offline: false,
        retryable: true,
      });
    }
  }

  readSession(): Promise<Result<WebSession, SessionFailure>> {
    return this.#controlPlane.readSession();
  }

  readReadiness(): ReturnType<ReadinessGateway['readReadiness']> {
    return this.#controlPlane.readReadiness();
  }

  async logout(): Promise<Result<void, SessionFailure>> {
    const remote = await this.#controlPlane.logout();
    const local = await this.#runtime.clearHumanSession();
    if (!remote.ok) {
      return remote;
    }
    return local.ok ? ok(undefined) : err(runtimeFailure(local.error));
  }
}

function runtimeFailure(failure: { readonly code: string; readonly retryable: boolean }): SessionFailure {
  return {
    boundary: 'identity',
    code: failure.code,
    offline: false,
    retryable: failure.retryable,
  };
}
