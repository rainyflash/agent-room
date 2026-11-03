import { describe, expect, it } from 'vitest';

import { shareExcerpt, shareTargets, withoutLink } from '@/features/room-share/domain/room-share';

const room = {
  description: 'Bring a crate error.\n\nWe read it together.',
  name: 'Rust help desk',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
};

describe('分享一间公开房间', () => {
  it('简介压成一行，按字截短；没写简介是 null', () => {
    expect(shareExcerpt(room.description, 100)).toBe('Bring a crate error. We read it together.');
    expect(shareExcerpt(room.description, 5)).toBe('Bring…');
    expect(shareExcerpt('读书会每周见', 3)).toBe('读书会…');
    expect(shareExcerpt('  \n ', 10)).toBeNull();
  });

  it('发到社交平台时链接另外带上，话里的那条去掉，免得出现两遍', () => {
    const text = `Come chat in “Rust help desk”\n${room.shareUrl}`;
    expect(withoutLink(text, room.shareUrl)).toBe('Come chat in “Rust help desk”');
    const links = shareTargets(room, text);
    const x = new URL(links.x);
    expect(x.origin + x.pathname).toBe('https://x.com/intent/post');
    expect(x.searchParams.get('text')).toBe('Come chat in “Rust help desk”');
    expect(x.searchParams.get('url')).toBe(room.shareUrl);
    const weibo = new URL(links.weibo);
    expect(weibo.host).toBe('service.weibo.com');
    expect(weibo.searchParams.get('title')).toBe('Come chat in “Rust help desk”');
    expect(weibo.searchParams.get('url')).toBe(room.shareUrl);
    const reddit = new URL(links.reddit);
    expect(reddit.host).toBe('www.reddit.com');
    expect(reddit.searchParams.get('url')).toBe(room.shareUrl);
  });

  it('话删光了用房间名；Reddit 的标题最多 300 个字', () => {
    expect(new URL(shareTargets(room, room.shareUrl).x).searchParams.get('text')).toBe(
      'Rust help desk',
    );
    const long = '字'.repeat(400);
    expect(new URL(shareTargets(room, long).reddit).searchParams.get('title')).toHaveLength(300);
  });
});
