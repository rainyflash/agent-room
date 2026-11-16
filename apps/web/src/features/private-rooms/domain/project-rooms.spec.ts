import { describe, expect, it } from 'vitest';

import { githubRepositorySchema, parseRepositoryInput } from './project-rooms';

describe('parseRepositoryInput', () => {
  it('认 owner/repo，也认从地址栏贴过来的 GitHub 地址', () => {
    expect(parseRepositoryInput('rainyflash/agent-room')).toBe('rainyflash/agent-room');
    expect(parseRepositoryInput('  rainyflash/agent-room  ')).toBe('rainyflash/agent-room');
    expect(parseRepositoryInput('https://github.com/rainyflash/agent-room')).toBe(
      'rainyflash/agent-room',
    );
    expect(parseRepositoryInput('github.com/rainyflash/agent-room.git')).toBe(
      'rainyflash/agent-room',
    );
    expect(parseRepositoryInput('https://www.github.com/Octo-Cat/hello_world.js/')).toBe(
      'Octo-Cat/hello_world.js',
    );
  });

  it('不合 GitHub 规矩的不认', () => {
    for (const value of [
      '',
      'agent-room',
      'a/b/c',
      '-owner/repo',
      'owner/..',
      'owner/.',
      'own er/repo',
      'owner/re po',
      `${'o'.repeat(40)}/repo`,
      `owner/${'r'.repeat(101)}`,
      'https://gitlab.com/owner/repo',
    ]) {
      expect(parseRepositoryInput(value)).toBeNull();
    }
  });

  it('回答里的仓库名照同一条规矩校验', () => {
    expect(githubRepositorySchema.safeParse('rainyflash/agent-room').success).toBe(true);
    expect(githubRepositorySchema.safeParse('rainyflash/agent-room/x').success).toBe(false);
  });
});
