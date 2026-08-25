export type RuntimeDependency =
  'bridge' | 'control_plane' | 'matrix' | 'object_storage' | 'oidc' | 'pixi';

export type ProductCapability =
  | 'agent_tools'
  | 'authenticate'
  | 'browse_lobby'
  | 'join_room'
  | 'open_content'
  | 'send_message'
  | 'visual_lobby';

export type CapabilityAvailability = 'available' | 'blocked' | 'read_only';

export type CapabilityDecision = {
  readonly capability: ProductCapability;
  readonly reasons: readonly RuntimeDependency[];
  readonly status: CapabilityAvailability;
};

type DependencyImpact = Partial<Readonly<Record<ProductCapability, CapabilityAvailability>>>;

const capabilities: readonly ProductCapability[] = Object.freeze([
  'browse_lobby',
  'join_room',
  'send_message',
  'open_content',
  'authenticate',
  'agent_tools',
  'visual_lobby',
]);

const dependencyImpacts: Readonly<Record<RuntimeDependency, DependencyImpact>> = Object.freeze({
  bridge: Object.freeze({ agent_tools: 'blocked' }),
  control_plane: Object.freeze({
    authenticate: 'blocked',
    browse_lobby: 'read_only',
    join_room: 'blocked',
    open_content: 'blocked',
  }),
  matrix: Object.freeze({
    agent_tools: 'blocked',
    browse_lobby: 'read_only',
    join_room: 'blocked',
    send_message: 'blocked',
  }),
  object_storage: Object.freeze({
    open_content: 'blocked',
    send_message: 'blocked',
  }),
  oidc: Object.freeze({ authenticate: 'blocked' }),
  pixi: Object.freeze({ visual_lobby: 'read_only' }),
});

const severity: Readonly<Record<CapabilityAvailability, number>> = Object.freeze({
  available: 0,
  read_only: 1,
  blocked: 2,
});

/**
 * 把依赖故障折叠成用户能力，而不是把“网络在线”冒充产品可用。
 * 未被故障命中的能力保持可用；多个故障同时发生时采用最严格结果。
 */
export function resolveDegradedCapabilities(
  unavailableDependencies: ReadonlySet<RuntimeDependency>,
): readonly CapabilityDecision[] {
  return capabilities.map((capability) => {
    const impacts = [...unavailableDependencies]
      .map((dependency) => ({ dependency, status: dependencyImpacts[dependency][capability] }))
      .filter(
        (
          impact,
        ): impact is {
          readonly dependency: RuntimeDependency;
          readonly status: CapabilityAvailability;
        } => impact.status !== undefined,
      );
    const status = impacts.reduce<CapabilityAvailability>(
      (current, impact) => (severity[impact.status] > severity[current] ? impact.status : current),
      'available',
    );
    const reasons = impacts
      .filter((impact) => impact.status === status)
      .map((impact) => impact.dependency)
      .sort();
    return Object.freeze({ capability, reasons: Object.freeze(reasons), status });
  });
}
