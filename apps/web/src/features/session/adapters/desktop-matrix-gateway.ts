import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import type {
  AuthenticationStartOutcome,
  MatrixGateway,
  MatrixRestoreOutcome,
  SessionFailure,
} from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';

export type DesktopMatrixGatewayOptions = {
  readonly matrix: MatrixAuthenticationSessionGateway;
  readonly runtime: Pick<DesktopRuntimeGateway, 'beginMatrixAuthentication'>;
};

export type MatrixAuthenticationSessionGateway = MatrixGateway & {
  exchangeAuthenticationGrant(
    loginToken: string,
    returnPath: string,
  ): Promise<Result<void, SessionFailure>>;
};

/**
 * 桌面层只接管系统浏览器与回环回调；Matrix 凭据交换、校验和连接仍由 Matrix 网关负责。
 */
export class DesktopMatrixGateway implements MatrixGateway {
  readonly #matrix: MatrixAuthenticationSessionGateway;
  readonly #runtime: Pick<DesktopRuntimeGateway, 'beginMatrixAuthentication'>;

  constructor({ matrix, runtime }: DesktopMatrixGatewayOptions) {
    this.#matrix = matrix;
    this.#runtime = runtime;
  }

  async beginAuthentication(
    returnPath: string,
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>> {
    const grant = await this.#runtime.beginMatrixAuthentication(returnPath);
    if (!grant.ok) {
      return err(runtimeFailure(grant.error));
    }
    const exchanged = await this.#matrix.exchangeAuthenticationGrant(
      grant.value.loginToken,
      grant.value.returnPath,
    );
    return exchanged.ok ? ok({ kind: 'session-established' }) : exchanged;
  }

  logout(): ReturnType<MatrixGateway['logout']> {
    return this.#matrix.logout();
  }

  restore(expectedUserId: string): Promise<Result<MatrixRestoreOutcome, SessionFailure>> {
    return this.#matrix.restore(expectedUserId);
  }
}

function runtimeFailure(failure: {
  readonly code: string;
  readonly retryable: boolean;
}): SessionFailure {
  return {
    boundary: 'matrix',
    code: failure.code,
    offline: false,
    retryable: failure.retryable,
  };
}
