import { describe, expect, it } from 'vitest';

import { controlPlaneEndpoint } from './control-plane-endpoint';

describe('Control Plane 端点解析', () => {
  it('对独立 API Origin 保持现有地址语义', () => {
    expect(controlPlaneEndpoint('https://api.agent-room.test', '/auth/session').toString()).toBe(
      'https://api.agent-room.test/auth/session',
    );
  });

  it('为浏览器同源 BFF 保留部署路径前缀', () => {
    expect(
      controlPlaneEndpoint(
        'https://app.agent-room.test/_agent-room/api',
        '/auth/oidc/start?intent=register',
      ).toString(),
    ).toBe('https://app.agent-room.test/_agent-room/api/auth/oidc/start?intent=register');
  });

  it.each(['/../session', '//evil.example/session'])('拒绝逃逸路径 %s', (path) => {
    expect(() => controlPlaneEndpoint('https://app.agent-room.test/_agent-room/api', path)).toThrow(
      TypeError,
    );
  });

  it.each([
    'file:///tmp/agent-room',
    'https://user:secret@app.agent-room.test/_agent-room/api',
    'https://app.agent-room.test/_agent-room/api?override=true',
  ])('拒绝不安全基址 %s', (baseUrl) => {
    expect(() => controlPlaneEndpoint(baseUrl, '/auth/session')).toThrow(TypeError);
  });
});
