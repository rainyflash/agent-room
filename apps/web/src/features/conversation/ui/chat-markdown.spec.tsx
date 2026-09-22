// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { ChatMarkdown } from './chat-markdown';
import { parseChatMarkdown } from '@/features/conversation/domain/chat-markdown';

afterEach(cleanup);

describe('聊天正文的受限 Markdown', () => {
  it('代码块、行内代码、粗体、列表和引用各成节点，纯文字段落保持原样', () => {
    const source = [
      '先跑这个：',
      '```bash',
      'cargo test -p agent-room-bridge',
      '```',
      '注意 `--locked` 参数，**别改锁文件**。',
      '',
      '- 第一步',
      '- 第二步',
      '1. 再来',
      '2. 结束',
      '> 引用一句',
      '## 小结',
      '两行',
      '连着',
    ].join('\n');
    const view = render(<ChatMarkdown source={source} />);

    const code = view.container.querySelector('pre.chat-markdown__code');
    expect(code).toHaveAttribute('data-language', 'bash');
    expect(code?.textContent).toBe('cargo test -p agent-room-bridge');
    expect(view.container.querySelector('p > code')?.textContent).toBe('--locked');
    expect(view.container.querySelector('strong')?.textContent).toBe('别改锁文件');
    expect([...view.container.querySelectorAll('ul > li')].map((item) => item.textContent)).toEqual(
      ['第一步', '第二步'],
    );
    expect([...view.container.querySelectorAll('ol > li')].map((item) => item.textContent)).toEqual(
      ['再来', '结束'],
    );
    expect(view.container.querySelector('blockquote')?.textContent).toBe('引用一句');
    expect(view.container.querySelector('.chat-markdown__heading')?.textContent).toBe('小结');
    // 段内换行保留为 <br>，不拆成多个段落。
    const last = view.container.querySelector('.chat-markdown > p:last-child');
    expect(last?.innerHTML).toBe('两行<br>连着');
  });

  it('没关上的代码块读到结尾，空消息不产生节点', () => {
    expect(parseChatMarkdown('```\nlet x = 1;\nlet y = 2;')).toEqual([
      { kind: 'code', language: '', text: 'let x = 1;\nlet y = 2;' },
    ]);
    expect(parseChatMarkdown('')).toEqual([]);
    expect(parseChatMarkdown('\n\n')).toEqual([]);
  });

  it('HTML 与链接只当文字，生成式攻击语料不产生任何可执行或可点击节点', () => {
    const view = render(<ChatMarkdown source={generatedAttackCorpus()} />);

    expect(
      view.container.querySelectorAll(
        'a, applet, audio, button, embed, form, iframe, img, input, link, meta, object, script, style, svg, video',
      ),
    ).toHaveLength(0);
    expect(Reflect.get(window, '__agentRoomCompromised')).toBeUndefined();
    expect(view.container.textContent).toContain('javascript:');
    expect(view.container.textContent).toContain('<script>');
  });
});

function generatedAttackCorpus(): string {
  const fragments = [
    '<script>window.__agentRoomCompromised=true</script>',
    '<img src=x onerror=window.__agentRoomCompromised=true>',
    '[run](javascript:window.__agentRoomCompromised=true)',
    '<iframe srcdoc="<script>alert(1)</script>"></iframe>',
    '`<svg><animate onbegin=alert(1)></animate></svg>`',
    '**<a href="javascript:alert(1)">x</a>**',
  ] as const;
  let state = 0x5a17c9e3;
  const lines = Array.from({ length: 256 }, (_, index) => {
    state = (Math.imul(state ^ (state >>> 15), 2_246_822_519) + 3_266_489_917) >>> 0;
    const fragment = fragments[state % fragments.length] ?? fragments[0];
    const prefix = ['# ', '## ', '- ', '1. ', '> ', '```', ''][index % 7] ?? '';
    return `${prefix}${fragment}-${state.toString(16)}`;
  });
  return lines.join('\n');
}
