const keepCurrentValue = (): void => undefined;

export function useRegisterSW() {
  return {
    needRefresh: [false, keepCurrentValue] as const,
    updateServiceWorker: (): Promise<void> => Promise.resolve(),
  };
}
