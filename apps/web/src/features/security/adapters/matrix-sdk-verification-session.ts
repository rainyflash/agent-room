import { VerificationMethod } from 'matrix-js-sdk/lib/types.js';
import {
  VerificationPhase,
  VerificationRequestEvent,
  VerifierEvent,
  type ShowSasCallbacks,
  type VerificationRequest,
  type Verifier,
} from 'matrix-js-sdk/lib/crypto-api/index.js';

import type {
  MatrixSecurityFailure,
  MatrixVerificationSas,
  MatrixVerificationSession,
  MatrixVerificationSnapshot,
} from '@/features/security/domain/matrix-security';
import { err, ok, type Result } from '@/shared/result';

export class MatrixSdkVerificationSession implements MatrixVerificationSession {
  readonly #listeners = new Set<() => void>();
  readonly #request: VerificationRequest;
  readonly #targetDeviceId: string | undefined;
  #disposed = false;
  #sasCallbacks: ShowSasCallbacks | null = null;
  #snapshot: MatrixVerificationSnapshot;
  #startingSas = false;
  #verifier: Verifier | null = null;

  constructor(request: VerificationRequest, targetDeviceId?: string) {
    this.#request = request;
    this.#targetDeviceId = targetDeviceId;
    this.#snapshot = waitingSnapshot(request, targetDeviceId);
  }

  activate(): void {
    this.#request.on(VerificationRequestEvent.Change, this.#handleRequestChange);
    this.#advanceRequest();
  }

  getSnapshot = (): MatrixVerificationSnapshot => this.#snapshot;

  subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  async confirm(): Promise<Result<void, MatrixSecurityFailure>> {
    if (this.#snapshot.stage !== 'comparing' || this.#sasCallbacks === null) {
      return err(verificationFailure('security.verification_unavailable', false));
    }
    const sas = this.#snapshot.sas;
    this.#publish(Object.freeze({ sas, stage: 'confirming' }));
    try {
      await this.#sasCallbacks.confirm();
      return ok(undefined);
    } catch {
      const failure = verificationFailure('security.verification_failed', true);
      this.#finish(Object.freeze({ failure, stage: 'failed' }));
      return err(failure);
    }
  }

  mismatch(): void {
    if (this.#sasCallbacks === null || this.#snapshot.stage !== 'comparing') {
      return;
    }
    this.#sasCallbacks.mismatch();
    this.#finish(Object.freeze({ cancellationCode: 'm.mismatched_sas', stage: 'cancelled' }));
  }

  async cancel(): Promise<Result<void, MatrixSecurityFailure>> {
    if (isTerminal(this.#snapshot)) {
      return ok(undefined);
    }
    try {
      await this.#request.cancel();
      this.#finish(Object.freeze({ cancellationCode: 'm.user', stage: 'cancelled' }));
      return ok(undefined);
    } catch {
      const failure = verificationFailure('security.verification_failed', true);
      this.#finish(Object.freeze({ failure, stage: 'failed' }));
      return err(failure);
    }
  }

  dispose(): void {
    this.#disposed = true;
    this.#detachEvents();
    this.#listeners.clear();
  }

  readonly #handleRequestChange = (): void => {
    this.#advanceRequest();
  };

  readonly #handleVerifierCancel = (): void => {
    this.#finish(
      Object.freeze({
        ...(this.#request.cancellationCode === null
          ? {}
          : { cancellationCode: this.#request.cancellationCode }),
        stage: 'cancelled',
      }),
    );
  };

  readonly #handleShowSas = (callbacks: ShowSasCallbacks): void => {
    const sas = projectSas(callbacks);
    if (sas === null) {
      const failure = verificationFailure('security.verification_failed', false);
      this.#finish(Object.freeze({ failure, stage: 'failed' }));
      return;
    }
    this.#sasCallbacks = callbacks;
    this.#publish(Object.freeze({ sas, stage: 'comparing' }));
  };

  #advanceRequest(): void {
    if (this.#disposed || isTerminal(this.#snapshot)) {
      return;
    }
    switch (this.#request.phase) {
      case VerificationPhase.Cancelled:
        this.#handleVerifierCancel();
        return;
      case VerificationPhase.Done:
        this.#finish(Object.freeze({ stage: 'verified' }));
        return;
      case VerificationPhase.Ready:
        this.#publish(waitingSnapshot(this.#request, this.#targetDeviceId));
        this.#startSas();
        return;
      case VerificationPhase.Started:
        this.#publish(waitingSnapshot(this.#request, this.#targetDeviceId));
        if (this.#request.verifier !== undefined) {
          this.#attachVerifier(this.#request.verifier);
        }
        return;
      case VerificationPhase.Requested:
      case VerificationPhase.Unsent:
        this.#publish(waitingSnapshot(this.#request, this.#targetDeviceId));
    }
  }

  #startSas(): void {
    if (this.#startingSas || this.#verifier !== null) {
      return;
    }
    this.#startingSas = true;
    void this.#request
      .startVerification(VerificationMethod.Sas)
      .then((verifier) => {
        this.#attachVerifier(verifier);
      })
      .catch(() => {
        const verifier = this.#request.verifier;
        if (verifier !== undefined) {
          this.#attachVerifier(verifier);
          return;
        }
        const failure = verificationFailure('security.verification_failed', true);
        this.#finish(Object.freeze({ failure, stage: 'failed' }));
      });
  }

  #attachVerifier(verifier: Verifier): void {
    if (this.#disposed || this.#verifier === verifier) {
      return;
    }
    this.#verifier = verifier;
    verifier.on(VerifierEvent.Cancel, this.#handleVerifierCancel);
    verifier.on(VerifierEvent.ShowSas, this.#handleShowSas);
    const existingSas = verifier.getShowSasCallbacks();
    if (existingSas !== null) {
      this.#handleShowSas(existingSas);
    }
    void verifier
      .verify()
      .then(() => {
        this.#finish(Object.freeze({ stage: 'verified' }));
      })
      .catch(() => {
        if (this.#request.phase === VerificationPhase.Cancelled || verifier.hasBeenCancelled) {
          this.#handleVerifierCancel();
          return;
        }
        const failure = verificationFailure('security.verification_failed', true);
        this.#finish(Object.freeze({ failure, stage: 'failed' }));
      });
  }

  #publish(snapshot: MatrixVerificationSnapshot): void {
    if (this.#disposed || Object.is(this.#snapshot, snapshot)) {
      return;
    }
    this.#snapshot = snapshot;
    for (const listener of this.#listeners) {
      listener();
    }
  }

  #finish(snapshot: MatrixVerificationSnapshot): void {
    if (isTerminal(this.#snapshot)) {
      return;
    }
    this.#publish(snapshot);
    this.#detachEvents();
  }

  #detachEvents(): void {
    this.#request.off(VerificationRequestEvent.Change, this.#handleRequestChange);
    this.#verifier?.off(VerifierEvent.Cancel, this.#handleVerifierCancel);
    this.#verifier?.off(VerifierEvent.ShowSas, this.#handleShowSas);
  }
}

function waitingSnapshot(
  request: VerificationRequest,
  targetDeviceId: string | undefined,
): MatrixVerificationSnapshot {
  return Object.freeze({
    stage: 'waiting',
    ...(targetDeviceId === undefined ? {} : { targetDeviceId }),
    ...(request.transactionId === undefined ? {} : { transactionId: request.transactionId }),
  });
}

function projectSas(callbacks: ShowSasCallbacks): MatrixVerificationSas | null {
  const decimals = callbacks.sas.decimal;
  const emojis = callbacks.sas.emoji?.map(([symbol, label]) => Object.freeze({ label, symbol }));
  if (decimals === undefined && emojis === undefined) {
    return null;
  }
  return Object.freeze({
    ...(decimals === undefined ? {} : { decimals: Object.freeze([...decimals]) }),
    ...(emojis === undefined ? {} : { emojis: Object.freeze(emojis) }),
  });
}

function isTerminal(snapshot: MatrixVerificationSnapshot): boolean {
  return (
    snapshot.stage === 'cancelled' || snapshot.stage === 'failed' || snapshot.stage === 'verified'
  );
}

function verificationFailure(
  code: MatrixSecurityFailure['code'],
  retryable: boolean,
): MatrixSecurityFailure {
  return Object.freeze({ code, retryable });
}
