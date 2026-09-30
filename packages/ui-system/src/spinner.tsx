import { LoaderCircle } from 'lucide-react';

import { classNames } from './class-names.js';

export type SpinnerProps = {
  /** 读屏读的文字；不传时只是装饰，旁边的文字已经说清在做什么。 */
  readonly label?: string;
  readonly className?: string;
};

/** 转圈的加载标记。动画在“减少动态效果”下由全局规则停掉。 */
export function Spinner({ label, className }: SpinnerProps) {
  return (
    <span
      aria-hidden={label === undefined ? true : undefined}
      aria-label={label}
      className={classNames('ar-spinner', className)}
      role={label === undefined ? undefined : 'status'}
    >
      <LoaderCircle aria-hidden="true" />
    </span>
  );
}
