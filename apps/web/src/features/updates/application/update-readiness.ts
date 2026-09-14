const guards = new Set<() => boolean>();

export function registerUpdateGuard(guard: () => boolean): () => void {
  guards.add(guard);
  return () => {
    guards.delete(guard);
  };
}
export function prepareForUpdate(): boolean {
  return [...guards].every((guard) => guard());
}
