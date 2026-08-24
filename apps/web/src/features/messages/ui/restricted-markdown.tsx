export type RestrictedMarkdownProps = {
  readonly source: string;
};

/**
 * 只把少量排版标记映射为 React 节点；HTML 与链接始终作为普通文本显示。
 */
export function RestrictedMarkdown({ source }: RestrictedMarkdownProps) {
  return (
    <div className="restricted-markdown">
      {source.split(/\r?\n/u).map((line, index) => renderLine(line, index))}
    </div>
  );
}

function renderLine(line: string, index: number) {
  const key = `${String(index)}-${line.slice(0, 16)}`;
  if (line.startsWith('### ')) {
    return <h4 key={key}>{line.slice(4)}</h4>;
  }
  if (line.startsWith('## ')) {
    return <h3 key={key}>{line.slice(3)}</h3>;
  }
  if (line.startsWith('# ')) {
    return <h2 key={key}>{line.slice(2)}</h2>;
  }
  if (/^[-*] /u.test(line)) {
    return (
      <p className="restricted-markdown__list-item" key={key}>
        {line.slice(2)}
      </p>
    );
  }
  if (line.startsWith('> ')) {
    return <blockquote key={key}>{line.slice(2)}</blockquote>;
  }
  if (line.trim().length === 0) {
    return <span aria-hidden="true" className="restricted-markdown__space" key={key} />;
  }
  return <p key={key}>{line}</p>;
}
