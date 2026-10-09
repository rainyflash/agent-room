import { describe, expect, it } from 'vitest';

import { siteOrigin } from './site-preview.js';

describe('siteOrigin', () => {
  it('控制面挂在网页域名下时，那个 Origin 就是网站', () => {
    expect(siteOrigin('https://app.agentroom.chat/_agent-room/api')).toBe(
      'https://app.agentroom.chat',
    );
    expect(siteOrigin('https://chat.example.org/_agent-room/api/')).toBe(
      'https://chat.example.org',
    );
  });

  it('控制面在自己的域名、没配或不是 https 时不知道网站在哪', () => {
    expect(siteOrigin('https://api.agentroom.chat')).toBeNull();
    expect(siteOrigin('https://api.agentroom.chat/')).toBeNull();
    expect(siteOrigin(undefined)).toBeNull();
    expect(siteOrigin('not a url')).toBeNull();
    expect(siteOrigin('http://app.agent-room.localhost/_agent-room/api')).toBeNull();
  });
});
