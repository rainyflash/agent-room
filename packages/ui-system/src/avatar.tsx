import type { ReactNode } from 'react';

import { classNames } from './class-names.js';

/**
 * `ink`：人，墨色的圆里一个字。`teal`、`coral`、`sunny`：垫在同色的机器人后面的浅底。
 * `plain`：浅灰底，给还不知道是谁的。
 */
export type AvatarTint = 'ink' | 'teal' | 'coral' | 'sunny' | 'plain';

export type AvatarProps = {
  readonly tint?: AvatarTint;
  /** 离线：整个头像变灰、变淡。 */
  readonly dimmed?: boolean;
  /** 右下角的记号，一般是 `PresenceDot`。 */
  readonly mark?: ReactNode;
  readonly className?: string | undefined;
  /** 圆里画的东西：一个字，或者一张画。读屏不读它，旁边的名字已经说了是谁。 */
  readonly children: ReactNode;
};

/** 头像：一个圆。大小跟着 `--ar-avatar-size`（默认 32px），在用它的地方定。 */
export function Avatar({ tint = 'plain', dimmed = false, mark, className, children }: AvatarProps) {
  return (
    <span
      className={classNames(
        'ar-avatar',
        `ar-avatar--${tint}`,
        dimmed && 'ar-avatar--dimmed',
        className,
      )}
    >
      <span aria-hidden="true" className="ar-avatar__face">
        {children}
      </span>
      {mark === undefined || mark === null ? null : <span className="ar-avatar__mark">{mark}</span>}
    </span>
  );
}
