// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { i18n, initializeI18n } from '@/shared/i18n/i18n';

import { RoomTitle } from './repository-mark';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(async () => {
  cleanup();
  await i18n.changeLanguage('en');
});

function renderTitle(repository: Parameters<typeof RoomTitle>[0]['repository']) {
  return render(
    <I18nextProvider i18n={i18n}>
      <RoomTitle name="agent-room" repository={repository} />
    </I18nextProvider>,
  );
}

describe('RoomTitle', () => {
  it('没绑仓库：只有房间名', () => {
    renderTitle(null);

    expect(screen.getByRole('heading', { level: 1, name: 'agent-room' })).toBeVisible();
    expect(screen.queryByRole('link')).toBeNull();
  });

  it('绑着仓库：房间名旁边一个勾，读屏和指上去都是“已验证的 GitHub 仓库”，新标签页打开 GitHub', () => {
    renderTitle({
      repository: 'rainyflash/agent-room',
      url: 'https://github.com/rainyflash/agent-room',
    });

    const mark = screen.getByRole('link', {
      name: 'Verified GitHub repository rainyflash/agent-room',
    });
    expect(mark).toHaveAttribute('href', 'https://github.com/rainyflash/agent-room');
    expect(mark).toHaveAttribute('target', '_blank');
    expect(mark).toHaveAttribute('rel', 'noopener noreferrer');
    expect(mark).toHaveAttribute('title', 'Verified GitHub repository rainyflash/agent-room');
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('agent-room');
  });

  it('中文：已验证的 GitHub 仓库 owner/repo', async () => {
    await i18n.changeLanguage('zh-CN');
    renderTitle({
      repository: 'rainyflash/agent-room',
      url: 'https://github.com/rainyflash/agent-room',
    });

    expect(
      screen.getByRole('link', { name: '已验证的 GitHub 仓库 rainyflash/agent-room' }),
    ).toBeVisible();
  });
});
