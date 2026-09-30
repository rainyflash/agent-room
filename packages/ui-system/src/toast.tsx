import { X } from 'lucide-react';
import type { ReactNode } from 'react';

import { Banner, type BannerTone } from './banner.js';
import { classNames } from './class-names.js';

export type ToastProps = {
  readonly tone?: BannerTone;
  /** 换掉默认图标。 */
  readonly icon?: ReactNode;
  readonly title?: ReactNode;
  readonly children?: ReactNode;
  /** 一两个按钮：去看、安装、接受之类。 */
  readonly action?: ReactNode;
  /** 给了才有关闭按钮。 */
  readonly onDismiss?: () => void;
  readonly dismissLabel?: string;
  /** 读屏怎么播报：默认危险用 alert，其余用 status。 */
  readonly role?: 'alert' | 'status';
  readonly className?: string;
};

/**
 * 右下角提示栈里的一条：新版本、新消息、别的设备请求核对都用它，外观和提示条一样，
 * 只是浮在页面上、可以关掉。
 */
export function Toast({
  tone = 'info',
  icon,
  title,
  children,
  action,
  onDismiss,
  dismissLabel,
  role,
  className,
}: ToastProps) {
  const dismiss =
    onDismiss === undefined ? null : (
      <button
        aria-label={dismissLabel}
        className="ar-icon-button ar-toast__dismiss"
        onClick={onDismiss}
        type="button"
      >
        <X aria-hidden="true" />
      </button>
    );
  return (
    <Banner
      action={
        action === undefined && dismiss === null ? undefined : (
          <>
            {action}
            {dismiss}
          </>
        )
      }
      className={classNames('ar-toast', className)}
      icon={icon}
      {...(role === undefined ? {} : { role })}
      title={title}
      tone={tone}
    >
      {children}
    </Banner>
  );
}

export type ToastStackProps = {
  /** 这一组提示的名字，读屏读它。 */
  readonly label: string;
  readonly children: ReactNode;
};

/** 右下角（手机上是底部）的提示栈：几条提示上下排开，不再互相压住。 */
export function ToastStack({ label, children }: ToastStackProps) {
  return (
    <section aria-label={label} className="ar-toast-stack">
      {children}
    </section>
  );
}
