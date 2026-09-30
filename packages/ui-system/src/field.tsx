import { cloneElement, useId, type ReactElement, type ReactNode } from 'react';

import { classNames } from './class-names.js';

type FieldControlProps = {
  readonly id?: string;
  readonly 'aria-describedby'?: string;
  readonly 'aria-invalid'?: boolean;
};

export type FieldProps = {
  readonly label: ReactNode;
  /** 输入框下面的一句说明。 */
  readonly hint?: ReactNode;
  /** 填得不对时的一句话；有它时输入框标成无效，读屏会读出来。 */
  readonly error?: ReactNode;
  /** 一个 input、textarea 或 select；标签、说明和错误都关联到它上面。 */
  readonly children: ReactElement<FieldControlProps>;
  readonly className?: string;
};

/**
 * 表单字段：标签在上，输入框在中间，说明和错误在下面。样式只按令牌来，各页面不再各写一套
 * 输入框。
 */
export function Field({ label, hint, error, children, className }: FieldProps) {
  const generatedId = useId();
  const hintId = useId();
  const errorId = useId();
  const invalid = error !== undefined && error !== null && error !== false;
  const controlId = children.props.id ?? generatedId;
  const describedBy = [hint === undefined ? null : hintId, invalid ? errorId : null]
    .filter((id) => id !== null)
    .join(' ');
  return (
    <div className={classNames('ar-field', invalid && 'ar-field--invalid', className)}>
      <label className="ar-field__label" htmlFor={controlId}>
        {label}
      </label>
      {cloneElement(children, {
        id: controlId,
        ...(describedBy === '' ? {} : { 'aria-describedby': describedBy }),
        ...(invalid ? { 'aria-invalid': true } : {}),
      })}
      {hint === undefined ? null : (
        <p className="ar-field__hint" id={hintId}>
          {hint}
        </p>
      )}
      {invalid ? (
        <p className="ar-field__error" id={errorId} role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
