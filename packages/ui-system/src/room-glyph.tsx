import { Lock } from 'lucide-react';

import { classNames } from './class-names.js';

/** `open`：能直接进的，是 `#`。`gated`：有门的，是锁。 */
export type RoomGlyphKind = 'open' | 'gated';

export type RoomGlyphProps = {
  readonly kind: RoomGlyphKind;
  /** 读屏读的那句话；旁边的字已经说清时不传，记号只是装饰。 */
  readonly label?: string | undefined;
  readonly className?: string | undefined;
};

/** 房间名字前面的记号，用大家认得的：`#` 和锁，不另造图标。 */
export function RoomGlyph({ kind, label, className }: RoomGlyphProps) {
  return (
    <span
      aria-hidden={label === undefined ? true : undefined}
      aria-label={label}
      className={classNames('ar-room-glyph', `ar-room-glyph--${kind}`, className)}
      role={label === undefined ? undefined : 'img'}
      title={label}
    >
      {kind === 'open' ? '#' : <Lock aria-hidden="true" />}
    </span>
  );
}
