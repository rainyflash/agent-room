import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { loadEnv, type Plugin } from 'vite';

// Bump for incompatible HTTP/client behavior that is not described by the event schema.
const writeEpoch = 1;
export const writeContract = createHash('sha256')
  .update(String(writeEpoch))
  .update(
    readFileSync(
      new URL('../../../packages/protocol/schema/v1/agent-room.schema.json', import.meta.url),
    ),
  )
  .digest('hex');

export function runtimeManifest(mode: string): Plugin {
  const env = loadEnv(mode, process.cwd(), 'VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL');
  const body = JSON.stringify({
    schema: 1,
    writeContract,
    windowsDownloadUrl:
      process.env.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL ??
      env.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL ??
      null,
  });
  return {
    name: 'agent-room-runtime-manifest',
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'runtime-manifest.json', source: body });
    },
    configureServer(server) {
      server.middlewares.use('/runtime-manifest.json', (_request, response) => {
        response.setHeader('Content-Type', 'application/json');
        response.setHeader('Cache-Control', 'no-store');
        response.end(body);
      });
    },
  };
}
