import { useRef, type KeyboardEvent, type ReactNode } from 'react';

import { classNames } from './class-names.js';

export type SegmentedOption<Value extends string> = {
  readonly value: Value;
  readonly label: ReactNode;
  readonly icon?: ReactNode;
};

export type SegmentedProps<Value extends string> = {
  /** 整组的名字，读屏读它。 */
  readonly label: string;
  readonly options: readonly SegmentedOption<Value>[];
  readonly value: Value;
  readonly onChange: (value: Value) => void;
  readonly className?: string;
};

const steps: Readonly<Record<string, number>> = {
  ArrowRight: 1,
  ArrowDown: 1,
  ArrowLeft: -1,
  ArrowUp: -1,
};

/**
 * 一行分段选择：几个互斥的选项，选中的那个是黄色按下态。键盘上和原生单选组一样，
 * 方向键在选项间移动并选中，Home / End 到头尾；只有选中的那个在 Tab 顺序里。
 */
export function Segmented<Value extends string>({
  label,
  options,
  value,
  onChange,
  className,
}: SegmentedProps<Value>) {
  const buttons = useRef(new Map<Value, HTMLButtonElement>());

  const move = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const step = steps[event.key];
    const target =
      event.key === 'Home'
        ? options[0]
        : event.key === 'End'
          ? options[options.length - 1]
          : step === undefined
            ? undefined
            : options[(index + step + options.length) % options.length];
    if (target === undefined) return;
    event.preventDefault();
    onChange(target.value);
    buttons.current.get(target.value)?.focus();
  };

  return (
    <div aria-label={label} className={classNames('ar-segmented', className)} role="radiogroup">
      {options.map((option, index) => (
        <button
          aria-checked={option.value === value}
          className="ar-segmented__option"
          key={option.value}
          onClick={() => {
            onChange(option.value);
          }}
          onKeyDown={(event) => {
            move(event, index);
          }}
          ref={(element) => {
            if (element === null) buttons.current.delete(option.value);
            else buttons.current.set(option.value, element);
          }}
          role="radio"
          tabIndex={option.value === value ? 0 : -1}
          type="button"
        >
          {option.icon === undefined ? null : (
            <span aria-hidden="true" className="ar-segmented__icon">
              {option.icon}
            </span>
          )}
          <span className="ar-segmented__label">{option.label}</span>
        </button>
      ))}
    </div>
  );
}
