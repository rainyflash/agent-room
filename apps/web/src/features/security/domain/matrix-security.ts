import type { Result } from '@/shared/result';

export type MatrixDeviceTrust = 'signed' | 'unknown' | 'unverified' | 'verified';

export type MatrixSecurityDevice = {
  readonly current: boolean;
  readonly deviceId: string;
  readonly displayName?: string;
  readonly fingerprint?: string;
  readonly trust: MatrixDeviceTrust;
  readonly userId: string;
};

/**
 * 你的设备，以及哪台是这台。签名全自动（ADR 0011），这里只读状态，不再有恢复密钥和核对。
 */
export type MatrixSecuritySnapshot = {
  readonly currentDeviceId: string;
  readonly devices: readonly MatrixSecurityDevice[];
  readonly userId: string;
};

export type MatrixSecurityFailure = {
  readonly code:
    | 'security.crypto_unavailable'
    | 'security.identity_unavailable'
    | 'security.inspection_failed'
    | 'security.matrix_unavailable';
  readonly retryable: boolean;
};

export type MatrixSecurityGateway = {
  inspect(): Promise<Result<MatrixSecuritySnapshot, MatrixSecurityFailure>>;
  /** Matrix 客户端换了或者有了新动静（比如这台设备刚签好）时通知，界面据此重读。 */
  subscribe(listener: () => void): () => void;
};
