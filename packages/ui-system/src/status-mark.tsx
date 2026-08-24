import { AlertTriangle, Check, CircleDashed, LoaderCircle, WifiOff } from 'lucide-react';
import type { ReactNode } from 'react';

import { classNames } from './class-names.js';

export type StatusTone = 'active' | 'alert' | 'idle' | 'network' | 'offline';

const iconByTone: Readonly<Record<StatusTone, ReactNode>> = {
  active: <Check aria-hidden="true" />,
  alert: <AlertTriangle aria-hidden="true" />,
  idle: <CircleDashed aria-hidden="true" />,
  network: <LoaderCircle aria-hidden="true" />,
  offline: <WifiOff aria-hidden="true" />,
};

export type StatusMarkProps = {
  readonly className?: string;
  readonly label: string;
  readonly pulse?: boolean;
  readonly tone: StatusTone;
};

export function StatusMark({ className, label, pulse = false, tone }: StatusMarkProps) {
  return (
    <span
      aria-label={label}
      className={classNames(
        'ar-status-mark',
        `ar-status-mark--${tone}`,
        pulse && 'ar-status-mark--pulse',
        className,
      )}
      role="img"
      title={label}
    >
      {iconByTone[tone]}
    </span>
  );
}
