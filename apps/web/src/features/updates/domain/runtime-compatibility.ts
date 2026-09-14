export type RuntimeWriteBlockReason = 'offline' | 'update_required';

export type RuntimeCompatibilityInput = {
  readonly online: boolean;
  readonly updateWaiting: boolean;
  readonly contractCompatible?: boolean;
};

export type RuntimeWriteAvailability =
  | { readonly allowed: true; readonly reason: null }
  | { readonly allowed: false; readonly reason: RuntimeWriteBlockReason };

type CompatibilityRule = {
  readonly reason: RuntimeWriteBlockReason;
  readonly violated: (input: RuntimeCompatibilityInput) => boolean;
};

const writeBlockRules: readonly CompatibilityRule[] = Object.freeze([
  {
    reason: 'update_required',
    violated: ({ updateWaiting, contractCompatible }) =>
      updateWaiting && contractCompatible !== true,
  },
  {
    reason: 'offline',
    violated: ({ online }) => !online,
  },
]);

/** Unknown and changed contracts remain read-only; compatible UI updates do not interrupt work. */
export function runtimeWriteAvailability(
  input: RuntimeCompatibilityInput,
): RuntimeWriteAvailability {
  const blocker = writeBlockRules.find((rule) => rule.violated(input));
  return blocker === undefined
    ? Object.freeze({ allowed: true, reason: null })
    : Object.freeze({ allowed: false, reason: blocker.reason });
}
