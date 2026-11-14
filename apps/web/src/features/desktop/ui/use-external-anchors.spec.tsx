// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useExternalAnchors } from './use-external-anchors';

afterEach(cleanup);

function Page({
  open,
  onInsideClick,
}: {
  readonly open: ((url: string) => Promise<boolean>) | undefined;
  readonly onInsideClick?: (event: React.MouseEvent) => void;
}) {
  useExternalAnchors(open);
  return (
    <main>
      <a href="https://github.com/rainyflash/agent-room/releases" rel="noreferrer" target="_blank">
        <span>Release notes</span>
      </a>
      <a href="/guide">Guide</a>
      <a href="mailto:someone@example.com" target="_blank">
        Mail
      </a>
      <a href="https://example.com/handled" onClick={onInsideClick} target="_blank">
        Handled
      </a>
    </main>
  );
}

/** 点一下，回浏览器默认的动作有没有被拦下。 */
function clickPrevented(element: Element, init: MouseEventInit = {}): boolean {
  return !fireEvent.click(element, init);
}

describe('桌面端页面里开新窗口的链接', () => {
  it('交给原生层打开，窗口自己不动；点在链接里面的字上也算', () => {
    const open = vi.fn(() => Promise.resolve(true));
    render(<Page open={open} />);

    expect(clickPrevented(screen.getByText('Release notes'))).toBe(true);

    expect(open).toHaveBeenCalledExactlyOnceWith(
      'https://github.com/rainyflash/agent-room/releases',
    );
  });

  it('应用里自己的链接、别的协议、不是左键的点击不管', () => {
    const open = vi.fn(() => Promise.resolve(true));
    render(<Page open={open} />);

    expect(clickPrevented(screen.getByText('Guide'))).toBe(false);
    expect(clickPrevented(screen.getByText('Mail'))).toBe(false);
    expect(clickPrevented(screen.getByText('Release notes'), { button: 1 })).toBe(false);

    expect(open).not.toHaveBeenCalled();
  });

  it('页面自己已经处理掉的点击不再打开一次', () => {
    const open = vi.fn(() => Promise.resolve(true));
    render(
      <Page
        onInsideClick={(event) => {
          event.preventDefault();
        }}
        open={open}
      />,
    );

    fireEvent.click(screen.getByText('Handled'));

    expect(open).not.toHaveBeenCalled();
  });

  it('网页端不接：链接照浏览器的来', () => {
    render(<Page open={undefined} />);

    expect(clickPrevented(screen.getByText('Release notes'))).toBe(false);
  });
});
