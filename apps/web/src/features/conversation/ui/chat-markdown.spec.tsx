// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { ChatMarkdown } from './chat-markdown';
import {
  chatMarkdownPlainText,
  parseChatMarkdown,
} from '@/features/conversation/domain/chat-markdown';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(cleanup);

function renderMarkdown(source: string) {
  return render(
    <I18nextProvider i18n={i18n}>
      <ChatMarkdown source={source} />
    </I18nextProvider>,
  );
}

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
    const view = renderMarkdown(source);

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

  it('只要字时去掉全部标记，HTML 和链接照旧是字', () => {
    expect(
      chatMarkdownPlainText(
        '## 小结\n- 跑 `pnpm test`\n- **别改锁文件**\n> 引用\n```\ncargo fmt\n```\n<b>[x](y)</b>',
      ),
    ).toBe('小结\n跑 pnpm test\n别改锁文件\n引用\ncargo fmt\n<b>[x](y)</b>');
    expect(chatMarkdownPlainText('')).toBe('');
  });

  it('HTML 与链接只当文字，生成式攻击语料不产生任何可执行或可点击节点', () => {
    const view = renderMarkdown(generatedAttackCorpus());

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

describe('管道线写的表格', () => {
  const text = (value: string) => [{ kind: 'text', text: value }];
  const release = [
    'Release status:',
    '| Step | Owner | Minutes |',
    '| :--- | :---: | ---: |',
    '| Build `web` | **Atlas** | 12 |',
    '| Deploy | Comet |',
    '| Verify | Nova | 3 | extra |',
    'Done after that.',
  ].join('\n');

  it('表头、分隔、几行格子：认成一张表，上面下面的字各是一段', () => {
    expect(parseChatMarkdown(release)).toEqual([
      { kind: 'paragraph', lines: [text('Release status:')] },
      {
        kind: 'table',
        align: ['left', 'center', 'right'],
        header: [text('Step'), text('Owner'), text('Minutes')],
        rows: [
          [
            [
              { kind: 'text', text: 'Build ' },
              { kind: 'code', text: 'web' },
            ],
            [{ kind: 'strong', text: 'Atlas' }],
            text('12'),
          ],
          // 少的格子补空，多出来的不要。
          [text('Deploy'), text('Comet'), []],
          [text('Verify'), text('Nova'), text('3')],
        ],
      },
      { kind: 'paragraph', lines: [text('Done after that.')] },
    ]);
  });

  it('两边不带竖线也认；没写冒号就不指定往哪边靠；反斜杠加竖线是格子里的字', () => {
    expect(parseChatMarkdown('a | b\n--- | ---\n1 \\| 2 | 3')).toEqual([
      {
        kind: 'table',
        align: [null, null],
        header: [text('a'), text('b')],
        rows: [[text('1 | 2'), text('3')]],
      },
    ]);
  });

  it('不是表格的照旧是字：没有分隔那一行、两行格子数不一样、列太多、写在代码块里', () => {
    const paragraph = (...lines: string[]) => [{ kind: 'paragraph', lines: lines.map(text) }];
    expect(parseChatMarkdown('a | b\nc | d')).toEqual(paragraph('a | b', 'c | d'));
    expect(parseChatMarkdown('a | b\n---')).toEqual(paragraph('a | b', '---'));
    expect(parseChatMarkdown('a | b\n--- | --- | ---')).toEqual(
      paragraph('a | b', '--- | --- | ---'),
    );
    const wide = Array.from({ length: 17 }, (_, index) => `c${String(index)}`).join(' | ');
    const rule = Array.from({ length: 17 }, () => '---').join(' | ');
    expect(parseChatMarkdown(`${wide}\n${rule}`)).toEqual(paragraph(wide, rule));
    expect(parseChatMarkdown('```\na | b\n--- | ---\n```')).toEqual([
      { kind: 'code', language: '', text: 'a | b\n--- | ---' },
    ]);
  });

  it('只有表头没有行也是一张表；表格后面跟着代码块时到那儿为止', () => {
    expect(parseChatMarkdown('| a |\n| - |')).toEqual([
      { kind: 'table', align: [null], header: [text('a')], rows: [] },
    ]);
    expect(parseChatMarkdown('| a |\n| - |\n| 1 |\n```\nx | y\n```')).toEqual([
      { kind: 'table', align: [null], header: [text('a')], rows: [[text('1')]] },
      { kind: 'code', language: '', text: 'x | y' },
    ]);
  });

  it('画成真的表格：表头是列标题，往哪边靠写在格子上；格子里的 HTML 还是字', () => {
    const view = renderMarkdown(
      `${release}\n\n| x |\n| - |\n| <img src=x onerror=alert(1)> [a](javascript:alert(1)) |`,
    );
    const [table, hostile] = screen.getAllByRole('table');
    expect(
      [...(table?.querySelectorAll('thead th') ?? [])].map((cell) => [
        cell.textContent,
        cell.getAttribute('scope'),
        cell.getAttribute('data-align'),
      ]),
    ).toEqual([
      ['Step', 'col', 'left'],
      ['Owner', 'col', 'center'],
      ['Minutes', 'col', 'right'],
    ]);
    expect(
      [...(table?.querySelectorAll('tbody tr') ?? [])].map((row) =>
        [...row.querySelectorAll('td')].map((cell) => cell.textContent),
      ),
    ).toEqual([
      ['Build web', 'Atlas', '12'],
      ['Deploy', 'Comet', ''],
      ['Verify', 'Nova', '3'],
    ]);
    expect(table?.querySelector('td code')?.textContent).toBe('web');
    expect(table?.querySelector('td strong')?.textContent).toBe('Atlas');
    expect(table?.querySelector('tbody td')).toHaveAttribute('data-align', 'left');
    expect(hostile?.querySelector('thead th')).not.toHaveAttribute('data-align');
    expect(hostile?.querySelector('td')?.textContent).toBe(
      '<img src=x onerror=alert(1)> [a](javascript:alert(1))',
    );
    expect(view.container.querySelectorAll('a, img, button, script')).toHaveLength(0);
  });

  it('放得下的表格不进键盘的顺序；比框宽了才能聚焦、才有名字', () => {
    const view = renderMarkdown(release);
    const frame = view.container.querySelector('.chat-markdown__table');
    expect(frame).not.toHaveAttribute('tabindex');
    expect(frame).not.toHaveAttribute('role');
    cleanup();

    Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains('chat-markdown__table') ? 900 : 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains('chat-markdown__table') ? 600 : 0;
      },
    });
    try {
      renderMarkdown(release);
      const scrollable = screen.getByRole('group', { name: 'Table, scrolls sideways' });
      expect(scrollable).toHaveAttribute('tabindex', '0');
      expect(scrollable).toContainElement(screen.getByRole('table'));
    } finally {
      Reflect.deleteProperty(HTMLElement.prototype, 'scrollWidth');
      Reflect.deleteProperty(HTMLElement.prototype, 'clientWidth');
    }
  });

  it('只要字时一行一行摆出来，竖线和分隔那一行不露出来', () => {
    expect(chatMarkdownPlainText(release)).toBe(
      [
        'Release status:',
        'Step  Owner  Minutes',
        'Build web  Atlas  12',
        'Deploy  Comet  ',
        'Verify  Nova  3',
        'Done after that.',
      ].join('\n'),
    );
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
