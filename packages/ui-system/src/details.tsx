import type { ReactNode } from 'react';

import { classNames } from './class-names.js';

export type DetailsProps = {
  readonly summary: ReactNode;
  readonly children: ReactNode;
  /** 只决定第一次显示时是否展开；之后由用户开合。 */
  readonly defaultOpen?: boolean;
  readonly className?: string;
};

/**
 * 默认收起的“详情”：排查用的标识、次要选项放在这里，主界面只留要做的事。
 * 用原生 `<details>`，键盘和读屏都不用额外处理。
 */
export function Details({ summary, children, defaultOpen = false, className }: DetailsProps) {
  return (
    <details className={classNames('ar-disclosure', 'ar-details', className)} open={defaultOpen}>
      <summary>{summary}</summary>
      <div className="ar-disclosure__body">{children}</div>
    </details>
  );
}
