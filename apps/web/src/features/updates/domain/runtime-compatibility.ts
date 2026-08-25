export type RuntimeWriteBlockReason = 'offline' | 'update_required';

export type RuntimeCompatibilityInput = {
  readonly online: boolean;
  readonly updateWaiting: boolean;
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
    violated: ({ updateWaiting }) => updateWaiting,
  },
  {
    reason: 'offline',
    violated: ({ online }) => !online,
  },
]);

/** 写入必须由当前页面与当前 Service Worker 共同解释；发现待激活版本后，旧页面只读。 */
export function runtimeWriteAvailability(
  input: RuntimeCompatibilityInput,
): RuntimeWriteAvailability {
  const blocker = writeBlockRules.find((rule) => rule.violated(input));
  return blocker === undefined
    ? Object.freeze({ allowed: true, reason: null })
    : Object.freeze({ allowed: false, reason: blocker.reason });
}
