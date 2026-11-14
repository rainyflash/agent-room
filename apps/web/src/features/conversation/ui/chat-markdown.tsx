import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  parseChatMarkdown,
  type MarkdownBlock,
  type MarkdownInline,
} from '@/features/conversation/domain/chat-markdown';
import { useExternalLinkRequest } from './external-link';

type Rendering = {
  readonly tableLabel: string;
  /** 正文里的地址点了交给谁；`null` 是没人接，地址只当字。 */
  readonly openLink: ((href: string) => void) | null;
};

/**
 * 聊天气泡里的正文：Agent 的回复常带代码块、列表和表格，纯文字显示读不了。
 * 只产生文字、粗体、行内代码、代码块、列表、引用和表格节点，永远没有图片或 HTML，也没有
 * 能直接打开的链接：直接写出来的地址只是一个能点的记号，点了交给外面的确认框
 * （`ExternalLinkGate`）；外面没有确认框时它就是字。
 */
export function ChatMarkdown({ source }: { readonly source: string }) {
  const { t } = useTranslation();
  const rendering: Rendering = {
    tableLabel: t('conversation.table'),
    openLink: useExternalLinkRequest(),
  };
  return (
    <div className="chat-markdown">
      {parseChatMarkdown(source).map((block, index) => renderBlock(block, index, rendering))}
    </div>
  );
}

/**
 * 表格放在自己能左右滚的框里，宽了不把消息那一列撑开。放不下时这个框才进键盘的顺序、
 * 才有名字：能滚的地方要能用键盘滚，放得下的表格不多占一次 Tab。
 */
function TableFrame({ label, children }: { readonly label: string; readonly children: ReactNode }) {
  const frame = useRef<HTMLDivElement>(null);
  const [overflowing, setOverflowing] = useState(false);
  useLayoutEffect(() => {
    const element = frame.current;
    if (element === null) return;
    const measure = (): void => {
      setOverflowing(element.scrollWidth > element.clientWidth + 1);
    };
    measure();
    // 没有这个接口的环境（jsdom）里只量这一次。
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    if (element.firstElementChild !== null) observer.observe(element.firstElementChild);
    return () => {
      observer.disconnect();
    };
  }, []);
  return (
    <div
      className="chat-markdown__table"
      ref={frame}
      {...(overflowing ? { role: 'group', 'aria-label': label, tabIndex: 0 } : {})}
    >
      {children}
    </div>
  );
}

function renderBlock(block: MarkdownBlock, index: number, rendering: Rendering) {
  const key = `${String(index)}-${block.kind}`;
  const renderInline = (inline: readonly MarkdownInline[]) =>
    renderInlinePieces(inline, rendering.openLink);
  const renderLines = (lines: readonly (readonly MarkdownInline[])[]) =>
    lines.map((line, lineIndex) => (
      <Fragment key={String(lineIndex)}>
        {lineIndex > 0 ? <br /> : null}
        {renderInline(line)}
      </Fragment>
    ));
  switch (block.kind) {
    case 'code':
      return (
        <pre className="chat-markdown__code" data-language={block.language || undefined} key={key}>
          <code>{block.text}</code>
        </pre>
      );
    case 'heading':
      // 气泡里的标题只是加粗的一行，不进文档大纲。
      return (
        <p className="chat-markdown__heading" data-level={block.level} key={key}>
          {renderInline(block.inline)}
        </p>
      );
    case 'list': {
      const items = block.items.map((item, itemIndex) => (
        <li key={`${key}-${String(itemIndex)}`}>{renderInline(item)}</li>
      ));
      return block.ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>;
    }
    case 'table':
      return (
        <TableFrame key={key} label={rendering.tableLabel}>
          <table>
            <thead>
              <tr>
                {block.header.map((cell, column) => (
                  <th
                    data-align={block.align[column] ?? undefined}
                    key={String(column)}
                    scope="col"
                  >
                    {renderInline(cell)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, rowIndex) => (
                <tr key={String(rowIndex)}>
                  {row.map((cell, column) => (
                    <td data-align={block.align[column] ?? undefined} key={String(column)}>
                      {renderInline(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </TableFrame>
      );
    case 'quote':
      return (
        <blockquote className="chat-markdown__quote" key={key}>
          {renderLines(block.lines)}
        </blockquote>
      );
    case 'paragraph':
      return <p key={key}>{renderLines(block.lines)}</p>;
  }
}

function renderInlinePieces(
  inline: readonly MarkdownInline[],
  openLink: ((href: string) => void) | null,
) {
  return inline.map((piece, index) => {
    const key = String(index);
    switch (piece.kind) {
      case 'code':
        return <code key={key}>{piece.text}</code>;
      case 'strong':
        return <strong key={key}>{piece.text}</strong>;
      case 'link':
        if (openLink === null) return <Fragment key={key}>{piece.text}</Fragment>;
        // 不是 `<a href>`：带地址的链接中键、Ctrl 加点、右键菜单都能绕过确认直接打开。
        return (
          <span
            className="chat-markdown__link"
            key={key}
            onClick={() => {
              openLink(piece.href);
            }}
            onKeyDown={(event) => {
              if (event.key !== 'Enter') return;
              event.preventDefault();
              openLink(piece.href);
            }}
            role="link"
            tabIndex={0}
          >
            {piece.text}
          </span>
        );
      case 'text':
        return <Fragment key={key}>{piece.text}</Fragment>;
    }
  });
}
