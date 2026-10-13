import { describe, expect, it } from 'vitest';
import { isExpectedHttpBoundary, matrixOrigin } from './http-boundary';

const workspacePath =
  '/_matrix/client/v3/user/%40new-user%3Amatrix.agent-room.localhost/account_data/io.github.rainyflash.agentroom.personal-workspace.v1';

describe('真实验收的 HTTP 边界', () => {
  it('新账号尚未写入个人工作区时允许标准的空状态响应', () => {
    expect(isExpectedHttpBoundary(404, `${matrixOrigin}${workspacePath}`, 'GET')).toBe(true);
  });

  it.each([401, 403, 429, 500, 503])('个人工作区的 HTTP %s 仍判为失败', (status) => {
    expect(isExpectedHttpBoundary(status, `${matrixOrigin}${workspacePath}`, 'GET')).toBe(false);
  });

  it.each(['POST', 'PUT', 'DELETE'])('不会忽略 %s 工作区失败', (method) => {
    expect(isExpectedHttpBoundary(404, `${matrixOrigin}${workspacePath}`, method)).toBe(false);
  });

  it('密钥备份里还没有某一把会话密钥时的 404 算正常，别的写法和状态照旧算失败', () => {
    const session =
      '/_matrix/client/v3/room_keys/keys/!zxYPnxbHLjLJyPJptT%3Amatrix.agent-room.localhost/dA4UcC%2Bx1AuF%2BK4Zxh5HuOn8R0PutsNapTFyTg5%2Bv5Q';
    expect(isExpectedHttpBoundary(404, `${matrixOrigin}${session}?version=1`, 'GET')).toBe(true);
    expect(isExpectedHttpBoundary(403, `${matrixOrigin}${session}?version=1`, 'GET')).toBe(false);
    expect(isExpectedHttpBoundary(404, `${matrixOrigin}${session}?version=1`, 'PUT')).toBe(false);
    for (const path of [
      '/_matrix/client/v3/room_keys/keys',
      '/_matrix/client/v3/room_keys/keys/!zxYPnxbHLjLJyPJptT%3Amatrix.agent-room.localhost',
      `${session}/extra`,
    ]) {
      expect(isExpectedHttpBoundary(404, `${matrixOrigin}${path}`, 'GET')).toBe(false);
    }
    expect(isExpectedHttpBoundary(404, `https://wrong.example${session}`, 'GET')).toBe(false);
  });

  it('不放过错误主机、未知类型和错误路由的 404', () => {
    for (const url of [
      `https://wrong.example${workspacePath}`,
      `${matrixOrigin}${workspacePath}.unknown`,
      `${matrixOrigin}${workspacePath}/extra`,
      `${matrixOrigin}/_matrix/client/v3/user/x/rooms/y/account_data/io.github.rainyflash.agentroom.personal-workspace.v1`,
    ]) {
      expect(isExpectedHttpBoundary(404, url, 'GET')).toBe(false);
    }
  });
});
