import { X } from 'lucide-react';
import { useId, useLayoutEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

import { classNames } from './class-names.js';

export type DialogSize = 'default' | 'wide';

export type DialogProps = {
  readonly title: ReactNode;
  readonly description?: ReactNode;
  /** 标题左边的小图标块；只是装饰，读屏读标题。 */
  readonly icon?: ReactNode;
  readonly closeLabel: string;
  readonly onClose: () => void;
  readonly size?: DialogSize;
  readonly footer?: ReactNode;
  readonly className?: string;
  readonly children: ReactNode;
};

/**
 * 模态对话框。原生 `<dialog>` 挂到 body 上，不继承打开它的容器的后代样式；Esc 和关闭按钮都走
 * `onClose`，关掉后焦点回到打开它的元素。没有 `showModal` 的环境（jsdom 等）退化为 `open` 属性，
 * 仍然可以访问。
 */
export function Dialog({
  title,
  description,
  icon,
  closeLabel,
  onClose,
  size = 'default',
  footer,
  className,
  children,
}: DialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();

  useLayoutEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    const trigger = document.activeElement;
    if (typeof element.showModal === 'function') element.showModal();
    else element.setAttribute('open', '');
    return () => {
      if (typeof element.close === 'function') element.close();
      else element.removeAttribute('open');
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus();
    };
  }, []);

  return createPortal(
    <dialog
      aria-describedby={description === undefined ? undefined : descriptionId}
      aria-labelledby={titleId}
      className={classNames('ar-dialog', size === 'wide' && 'ar-dialog--wide', className)}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      ref={dialog}
    >
      <header className="ar-dialog__header">
        {icon === undefined ? null : (
          <span aria-hidden="true" className="ar-dialog__icon">
            {icon}
          </span>
        )}
        <div className="ar-dialog__heading">
          <h2 id={titleId}>{title}</h2>
          {description === undefined ? null : <p id={descriptionId}>{description}</p>}
        </div>
        <button
          aria-label={closeLabel}
          className="ar-icon-button ar-dialog__close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="ar-dialog__body">{children}</div>
      {footer === undefined ? null : <footer className="ar-dialog__footer">{footer}</footer>}
    </dialog>,
    document.body,
  );
}
