import { describe, expect, it } from 'vitest';

import {
  avatarLetter,
  avatarTone,
  exploreAvatarTones,
  exploreFilterFromSearch,
  exploreJoinPath,
  exploreSearchFromFilter,
  exploreSearchParams,
  joinContinuationStep,
  normalizeExploreSearch,
  type ExploreRoom,
} from '@/features/explore/domain/explore';

describe('发现页的搜索和筛选', () => {
  it('快捷筛选换成服务器认的参数，“全部”和最活跃不带', () => {
    expect(exploreSearchParams({ q: '', quick: 'all', sort: 'active' }, null).toString()).toBe('');
    expect(exploreSearchParams({ q: '', quick: 'open', sort: 'active' }, null).toString()).toBe(
      'join=open',
    );
    expect(exploreSearchParams({ q: '', quick: 'active', sort: 'newest' }, null).toString()).toBe(
      'active=true&sort=newest',
    );
    expect(
      exploreSearchParams({ q: ' 读书 ', quick: 'plain', sort: 'active' }, '40').toString(),
    ).toBe('q=%E8%AF%BB%E4%B9%A6&encrypted=false&cursor=40');
  });

  it('地址上认不出的值当默认的，默认的不写进地址', () => {
    expect(normalizeExploreSearch({ q: '  ', quick: 'all', sort: 'active' })).toEqual({});
    expect(normalizeExploreSearch({ q: 'rust', quick: 'nope', sort: 7 })).toEqual({ q: 'rust' });
    expect(
      normalizeExploreSearch({ q: 'x'.repeat(150), quick: 'request', sort: 'members' }),
    ).toEqual({ q: 'x'.repeat(100), quick: 'request', sort: 'members' });
    const filter = { q: 'rust', quick: 'plain', sort: 'newest' } as const;
    expect(exploreFilterFromSearch(exploreSearchFromFilter(filter))).toEqual(filter);
    expect(exploreFilterFromSearch({})).toEqual({ q: '', quick: 'all', sort: 'active' });
  });
});

describe('从分享页点“加入”过来', () => {
  it('地址上认 join=1，signup 只跟着 join 一起才算；换筛选时不带它们', () => {
    expect(normalizeExploreSearch({ join: 1 })).toEqual({ join: 1 });
    expect(normalizeExploreSearch({ join: '1', signup: 1 })).toEqual({ join: 1, signup: 1 });
    expect(normalizeExploreSearch({ signup: 1 })).toEqual({});
    expect(normalizeExploreSearch({ join: 2 })).toEqual({});
    expect(exploreSearchFromFilter(exploreFilterFromSearch({ join: 1, q: 'rust' }))).toEqual({
      q: 'rust',
    });
    expect(exploreJoinPath('aB3xYz')).toBe('/explore/aB3xYz?join=1');
  });

  it('登录回来：在里面的进房间，直接进的和受邀的加入，要申请的去留话，别的不动', () => {
    const base: ExploreRoom = {
      agentCount: 0,
      catalogId: '0198c000-0000-7000-8000-000000000001',
      createdAtUnixMs: 0,
      description: '',
      endToEndEncrypted: false,
      joinMode: 'open',
      lastMessageAtUnixMs: null,
      memberCount: 1,
      name: 'Room',
      ownerName: 'Ada',
      shareCode: 'aB3xYz',
      shareUrl: 'https://agentroom.test/r/aB3xYz',
      viewer: null,
    };
    expect(joinContinuationStep({ ...base, viewer: 'joined' })).toBe('enter');
    expect(joinContinuationStep(base)).toBe('join');
    expect(joinContinuationStep({ ...base, joinMode: 'invite_only', viewer: 'invited' })).toBe(
      'join',
    );
    expect(joinContinuationStep({ ...base, joinMode: 'request' })).toBe('request');
    expect(joinContinuationStep({ ...base, joinMode: 'request', viewer: 'pending' })).toBe('none');
    expect(joinContinuationStep({ ...base, viewer: 'banned' })).toBe('none');
    expect(joinContinuationStep({ ...base, joinMode: 'invite_only' })).toBe('none');
  });
});

describe('房间的头像', () => {
  it('写名字的第一个字，带肤色的表情算一个', () => {
    expect(avatarLetter('  rust help')).toBe('R');
    expect(avatarLetter('周末读书会')).toBe('周');
    expect(avatarLetter('👋🏽 hello')).toBe('👋🏽');
    expect(avatarLetter('   ')).toBe('#');
  });

  it('底色按房间号挑，同一间房总是同一个', () => {
    const id = '0198c000-0000-7000-8000-000000000001';
    expect(avatarTone(id)).toBe(avatarTone(id));
    expect(exploreAvatarTones).toContain(avatarTone(id));
    const tones = new Set(
      Array.from({ length: 40 }, (_, index) =>
        avatarTone(`0198c000-0000-7000-8000-${index.toString(16).padStart(12, '0')}`),
      ),
    );
    expect(tones.size).toBeGreaterThan(1);
  });
});
