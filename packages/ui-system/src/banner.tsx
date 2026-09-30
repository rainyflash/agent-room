import { AlertTriangle, CircleCheckBig, Info, OctagonAlert } from 'lucide-react';
import type { ReactNode } from 'react';

import { classNames } from './class-names.js';

export type BannerTone = 'danger' | 'info' | 'success' | 'warning';

const iconByTone: Readonly<Record<BannerTone, ReactNode>> = {
  danger: <OctagonAlert aria-hidden="true" />,
  info: <Info aria-hidden="true" />,
  success: <CircleCheckBig aria-hidden="true" />,
  warning: <AlertTriangle aria-hidden="true" />,
};

export type BannerProps = {
  readonly tone?: BannerTone;
  readonly title?: ReactNode;
  readonly children?: ReactNode;
  /** 右侧（手机上是下方）的一个操作，通常是一个按钮。 */
  readonly action?: ReactNode;
  /** 换掉默认图标；传 null 不显示图标。 */
  readonly icon?: ReactNode;
  /** 读屏怎么播报：默认危险用 alert，其余用 status；null 表示不是实时区域。 */
  readonly role?: 'alert' | 'status' | null;
  readonly className?: string;
};

/**
 * 页面或对话框里的一条提示：图标、一句标题、一两句说明和至多一个操作。颜色只是辅助，
 * 图标和文字本身要能说清是提示、成功、需要处理还是出错。
 */
export function Banner({
  tone = 'info',
  title,
  children,
  action,
  icon,
  role,
  className,
}: BannerProps) {
  const liveRole = role === undefined ? (tone === 'danger' ? 'alert' : 'status') : role;
  return (
    <div
      className={classNames('ar-banner', `ar-banner--${tone}`, className)}
      role={liveRole ?? undefined}
    >
      {icon === null ? null : (
        <span aria-hidden="true" className="ar-banner__icon">
          {icon ?? iconByTone[tone]}
        </span>
      )}
      <div className="ar-banner__content">
        {title === undefined ? null : <strong className="ar-banner__title">{title}</strong>}
        {children === undefined ? null : <div className="ar-banner__text">{children}</div>}
      </div>
      {action === undefined ? null : <div className="ar-banner__action">{action}</div>}
    </div>
  );
}
