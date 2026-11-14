import type { ReactNode } from 'react';

import { classNames } from './class-names.js';

export type DetailsProps = {
  readonly summary: ReactNode;
  readonly children: ReactNode;
  /** 只决定第一次显示时是否展开；之后由用户开合。 */
  readonly defaultOpen?: boolean;
  readonly className?: string;
  /** 开合时说一声：里面的东西要打开了才去读时用。 */
  readonly onToggle?: (open: boolean) => void;
};

/**
 * 默认收起的“详情”：排查用的标识、次要选项放在这里，主界面只留要做的事。
 * 用原生 `<details>`，键盘和读屏都不用额外处理。
 */
export function Details({
  summary,
  children,
  defaultOpen = false,
  className,
  onToggle,
}: DetailsProps) {
  return (
    <details
      className={classNames('ar-disclosure', 'ar-details', className)}
      onToggle={
        onToggle === undefined
          ? undefined
          : (event) => {
              onToggle(event.currentTarget.open);
            }
      }
      open={defaultOpen}
    >
      <summary>{summary}</summary>
      <div className="ar-disclosure__body">{children}</div>
    </details>
  );
}
