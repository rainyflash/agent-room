import type {
  AuthenticationStartOutcome,
  MatrixGateway,
  MatrixRestoreOutcome,
  SessionFailure,
} from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';
import { supersededMatrixSession } from '../domain/matrix-session-repository';
import type { DesktopMatrixSessions } from './control-plane-desktop-matrix-sessions';

export type DesktopMatrixGatewayOptions = {
  readonly matrix: MatrixIssuedSessionGateway;
  readonly sessions: DesktopMatrixSessions;
};

export type MatrixIssuedSessionGateway = MatrixGateway & {
  adoptIssuedSession(login: unknown, returnPath: string): Promise<Result<void, SessionFailure>>;
};

/**
 * 桌面端的聊天登录由控制面替人发（`specs/desktop-sign-in/design.md`），不开系统浏览器；拿到以后的
 * 校验、存储和连接仍由 Matrix 网关负责。
 */
export class DesktopMatrixGateway implements MatrixGateway {
  readonly #matrix: MatrixIssuedSessionGateway;
  readonly #sessions: DesktopMatrixSessions;
  #authenticationEpoch = 0;

  constructor({ matrix, sessions }: DesktopMatrixGatewayOptions) {
    this.#matrix = matrix;
    this.#sessions = sessions;
  }

  async beginAuthentication(
    returnPath: string,
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>> {
    const epoch = this.#authenticationEpoch;
    const issued = await this.#sessions.issue();
    // 等的时候退出登录或断开了：迟到的登录不要了。
    if (epoch !== this.#authenticationEpoch) return err(supersededMatrixSession());
    if (!issued.ok) return issued;
    const adopted = await this.#matrix.adoptIssuedSession(issued.value, returnPath);
    return adopted.ok ? ok({ kind: 'session-established' }) : adopted;
  }

  logout(): ReturnType<MatrixGateway['logout']> {
    this.#authenticationEpoch += 1;
    return this.#matrix.logout();
  }

  disconnect(): void {
    this.#authenticationEpoch += 1;
    this.#matrix.disconnect();
  }

  restore(expectedUserId: string): Promise<Result<MatrixRestoreOutcome, SessionFailure>> {
    return this.#matrix.restore(expectedUserId);
  }
}
