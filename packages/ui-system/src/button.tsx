import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';

import { classNames } from './class-names.js';

/** `send` 是发送消息专用的珊瑚色（游戏大厅配色：珊瑚色 = 发送、未读、需要注意）。 */
export type ButtonTone = 'alert' | 'ghost' | 'network' | 'primary' | 'quiet' | 'send';
export type ButtonSize = 'compact' | 'default' | 'large';

const toneClass: Readonly<Record<ButtonTone, string>> = {
  alert: 'ar-button--alert',
  ghost: 'ar-button--ghost',
  network: 'ar-button--network',
  primary: 'ar-button--primary',
  quiet: 'ar-button--quiet',
  send: 'ar-button--send',
};

const sizeClass: Readonly<Record<ButtonSize, string>> = {
  compact: 'ar-button--compact',
  default: 'ar-button--default',
  large: 'ar-button--large',
};

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  readonly icon?: ReactNode;
  readonly size?: ButtonSize;
  readonly tone?: ButtonTone;
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    children,
    className,
    icon,
    size = 'default',
    tone = 'primary',
    type = 'button',
    ...buttonProps
  },
  ref,
) {
  return (
    <button
      className={classNames('ar-button', toneClass[tone], sizeClass[size], className)}
      ref={ref}
      type={type}
      {...buttonProps}
    >
      {icon === undefined ? null : <span className="ar-button__icon">{icon}</span>}
      <span>{children}</span>
    </button>
  );
});
