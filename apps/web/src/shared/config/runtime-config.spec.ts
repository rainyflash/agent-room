import { describe, expect, it } from 'vitest';

import { loadRuntimeConfig } from './runtime-config';

const requiredEnvironment = {
  VITE_AGENT_ROOM_CONTROL_PLANE_URL: 'https://api.agent-room.test',
  VITE_AGENT_ROOM_MATRIX_HOMESERVER_URL: 'https://matrix.agent-room.test',
};

describe('运行时配置', () => {
  it('下载地址缺失时显式返回不可用而不是生成死链', () => {
    const result = loadRuntimeConfig(requiredEnvironment);

    expect(result).toEqual({
      ok: true,
      value: {
        controlPlaneUrl: 'https://api.agent-room.test',
        matrixHomeserverUrl: 'https://matrix.agent-room.test',
        windowsDownloadUrl: null,
      },
    });
  });

  it('空下载地址与未配置具有相同语义', () => {
    const result = loadRuntimeConfig({
      ...requiredEnvironment,
      VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL: '',
    });

    expect(result.ok && result.value.windowsDownloadUrl).toBeNull();
  });

  it('只接受显式的有效下载 URL', () => {
    const result = loadRuntimeConfig({
      ...requiredEnvironment,
      VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL: 'not-a-url',
    });

    expect(result.ok).toBe(false);
  });
});
