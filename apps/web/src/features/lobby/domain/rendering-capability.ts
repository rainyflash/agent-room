export type LobbyRenderingCapability = {
  readonly compactViewport: boolean;
  readonly deviceMemoryGiB: number | null;
  readonly forcedColors: boolean;
  readonly hardwareConcurrency: number | null;
};

export type ListModeRequirement = 'compact' | 'constrained_device' | 'forced_colors' | null;

export function assessListModeRequirement(
  capability: LobbyRenderingCapability,
): ListModeRequirement {
  if (capability.compactViewport) {
    return 'compact';
  }
  if (capability.forcedColors) {
    return 'forced_colors';
  }
  if (
    (capability.deviceMemoryGiB !== null && capability.deviceMemoryGiB <= 2) ||
    (capability.deviceMemoryGiB === null &&
      capability.hardwareConcurrency !== null &&
      capability.hardwareConcurrency <= 2)
  ) {
    return 'constrained_device';
  }
  return null;
}
