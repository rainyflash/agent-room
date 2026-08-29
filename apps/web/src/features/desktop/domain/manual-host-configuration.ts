import type { ManualHostConfiguration } from '@/features/desktop/domain/desktop-runtime';

export function serializeManualHostConfiguration(config: ManualHostConfiguration): string {
  return JSON.stringify(
    {
      mcpServers: {
        [config.serverName]: {
          args: config.args,
          command: config.command,
          type: config.transport,
        },
      },
    },
    null,
    2,
  );
}
