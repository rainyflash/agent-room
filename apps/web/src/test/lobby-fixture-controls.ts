import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';

/** 仅由隔离的浏览器测试入口提供，生产应用不注册此接口。 */
export type LobbyFixtureControls = {
  receive(input: {
    readonly text: string;
    readonly agentIndex?: number;
    readonly human?: boolean;
    readonly roomId?: string;
    readonly ageMs?: number;
  }): string;
  redact(messageId: string): void;
  joinAgent(): string;
  leaveAgent(agentId: string): void;
};
export type LobbyFixtureWindow = Window &
  typeof globalThis & {
    readonly __agentRoomFixtureControls: LobbyFixtureControls;
    readonly __agentRoomFixtureScene: LobbySceneProjection;
    readonly __agentRoomFixtureContentReads: {
      readonly downloads: number;
      readonly tickets: number;
    };
  };
