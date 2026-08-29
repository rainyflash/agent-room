import { describe, expect, it } from 'vitest';

import { serializeManualHostConfiguration } from './manual-host-configuration';

describe('通用 MCP 宿主配置', () => {
  it('只序列化已验证的 STDIO 边界，不猜测宿主私有字段', () => {
    expect(
      JSON.parse(
        serializeManualHostConfiguration({
          args: [],
          command: 'C:\\Agent Room\\agent-room-mcp.exe',
          serverName: 'agent_room',
          transport: 'stdio',
        }),
      ),
    ).toEqual({
      mcpServers: {
        agent_room: {
          args: [],
          command: 'C:\\Agent Room\\agent-room-mcp.exe',
          type: 'stdio',
        },
      },
    });
  });
});
