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
