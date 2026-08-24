export type Result<TValue, TError> =
  { readonly ok: true; readonly value: TValue } | { readonly error: TError; readonly ok: false };

export function ok<TValue>(value: TValue): Result<TValue, never> {
  return { ok: true, value };
}

export function err<TError>(error: TError): Result<never, TError> {
  return { error, ok: false };
}
