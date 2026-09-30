import { AlertTriangle, Check, Copy } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { Button, type ButtonSize, type ButtonTone } from './button.js';
import { classNames } from './class-names.js';

export type CopyBlockProps = {
  /** 要复制的原文。 */
  readonly text: string;
  readonly copyLabel: string;
  readonly copiedLabel: string;
  /** 复制失败时的说明：原文还在上面，可以手动选中复制。 */
  readonly failedLabel: string;
  /** 原文区域的名字，读屏读它。 */
  readonly textLabel?: string;
  /** 不显示原文，只留按钮（原文另有地方显示时）。 */
  readonly hideText?: boolean;
  readonly tone?: ButtonTone;
  readonly size?: ButtonSize;
  /** 还不能发（比如这台电脑还没授权）：原文照样显示，按钮先灰着。 */
  readonly disabled?: boolean;
  readonly onCopied?: (() => void) | undefined;
  readonly className?: string;
};

type CopyResult = { readonly text: string; readonly state: 'copied' | 'failed' };

/**
 * 一段要发给别人的话和一个复制按钮。复制成功后按钮换成“已复制”；原文改了就回到初始状态，
 * 不会让旧的“已复制”留在新内容旁边。
 */
export function CopyBlock({
  text,
  copyLabel,
  copiedLabel,
  failedLabel,
  textLabel,
  hideText = false,
  tone = 'primary',
  size = 'large',
  disabled = false,
  onCopied,
  className,
}: CopyBlockProps) {
  const [result, setResult] = useState<CopyResult | null>(null);
  const generation = useRef(0);
  useEffect(
    () => () => {
      generation.current += 1;
    },
    [],
  );
  const state = result?.text === text ? result.state : 'idle';

  const copy = async (): Promise<void> => {
    const current = ++generation.current;
    try {
      await navigator.clipboard.writeText(text);
      if (current !== generation.current) return;
      setResult({ text, state: 'copied' });
      onCopied?.();
    } catch {
      if (current === generation.current) setResult({ text, state: 'failed' });
    }
  };

  return (
    <div className={classNames('ar-copy-block', className)}>
      {hideText ? null : (
        <pre aria-label={textLabel} className="ar-copy-block__text" tabIndex={0}>
          {text}
        </pre>
      )}
      <Button
        disabled={disabled}
        icon={state === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
        onClick={() => void copy()}
        size={size}
        tone={state === 'failed' ? 'alert' : tone}
      >
        {state === 'copied' ? copiedLabel : copyLabel}
      </Button>
      {state === 'failed' ? (
        <p className="ar-copy-block__failed" role="status">
          <AlertTriangle aria-hidden="true" />
          {failedLabel}
        </p>
      ) : null}
    </div>
  );
}
