import { classNames } from './class-names.js';

/**
 * `waiting`：在等消息，绿色实心。`wakes`：没在等，但叫得醒，绿色的圈。
 * 连着却没在等的不画点；离线的把头像变灰（`Avatar` 的 `dimmed`），也不画点。
 */
export type PresenceDotKind = 'waiting' | 'wakes';

export type PresenceDotProps = {
  readonly kind: PresenceDotKind;
  /** 读屏读的那句话；旁边已经用字写了状态时不传，点只是装饰。 */
  readonly label?: string | undefined;
  readonly className?: string | undefined;
};

/** 状态的点：界面上除了机器人，只有它是绿的。 */
export function PresenceDot({ kind, label, className }: PresenceDotProps) {
  return (
    <span
      aria-hidden={label === undefined ? true : undefined}
      aria-label={label}
      className={classNames('ar-presence-dot', `ar-presence-dot--${kind}`, className)}
      role={label === undefined ? undefined : 'img'}
      title={label}
    />
  );
}
