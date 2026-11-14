// @vitest-environment jsdom

import { cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { useUnreadTitle } from './use-unread-title';

function Title({ count }: { readonly count: number }) {
  useUnreadTitle(count);
  return null;
}

beforeEach(() => {
  document.title = 'Agent Room';
});
afterEach(cleanup);

describe('窗口标题上的条数', () => {
  it('有没看到的写在前面，变了跟着变，没有了改回去', () => {
    const view = render(<Title count={0} />);
    expect(document.title).toBe('Agent Room');
    view.rerender(<Title count={3} />);
    expect(document.title).toBe('(3) Agent Room');
    view.rerender(<Title count={4} />);
    expect(document.title).toBe('(4) Agent Room');
    view.rerender(<Title count={120} />);
    expect(document.title).toBe('(99+) Agent Room');
    view.rerender(<Title count={0} />);
    expect(document.title).toBe('Agent Room');
  });

  it('离开这一页时改回去', () => {
    const view = render(<Title count={2} />);
    expect(document.title).toBe('(2) Agent Room');
    view.unmount();
    expect(document.title).toBe('Agent Room');
  });
});
