import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { ChevronDown, RadioTower } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ConnectionStage } from '@/features/session/ui/connection-model';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import type { TranslationKey } from '@/shared/i18n/resources';

export type ConnectionRailProps = {
  readonly sessionKey?: TranslationKey;
  readonly stages: readonly ConnectionStage[];
  readonly transportKey?: TranslationKey;
};

const toneByStage: Readonly<Record<ConnectionStage['status'], StatusTone>> = {
  blocked: 'alert',
  complete: 'active',
  current: 'network',
  pending: 'idle',
};

const statusMessageKey: Readonly<Record<ConnectionStage['status'], TranslationKey>> = {
  blocked: 'connection.stage.status.blocked',
  complete: 'connection.stage.status.complete',
  current: 'connection.stage.status.current',
  pending: 'connection.stage.status.pending',
};

export function ConnectionRail({
  sessionKey = 'connection.session',
  stages,
  transportKey = 'connection.transport',
}: ConnectionRailProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const progressId = useId();
  const reduceMotion = useReducedMotion();
  return (
    <aside className="connection-rail" data-expanded={expanded}>
      <header className="connection-rail__header">
        <a className="brand-lockup" href="/connect">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <LanguageControl />
      </header>

      <div className="connection-rail__intro">
        <RadioTower aria-hidden="true" />
        <p>{t('app.environment')}</p>
      </div>

      <button
        aria-controls={progressId}
        aria-expanded={expanded}
        className="connection-progress-toggle"
        onClick={() => {
          setExpanded((previous) => !previous);
        }}
        type="button"
      >
        {t(expanded ? 'connection.progress.hide' : 'connection.progress.show')}
        <motion.span
          animate={{ rotate: expanded ? 180 : 0 }}
          transition={
            reduceMotion === true
              ? { duration: 0 }
              : { type: 'spring', stiffness: 280, damping: 26 }
          }
        >
          <ChevronDown aria-hidden="true" size={16} />
        </motion.span>
      </button>

      <ol aria-label={t('connection.progress')} className="connection-steps" id={progressId}>
        {stages.map((stage) => (
          <li className={`connection-step connection-step--${stage.status}`} key={stage.titleKey}>
            <div className="connection-step__index">
              <span>{String(stage.index + 1).padStart(2, '0')}</span>
              <StatusMark
                label={t(statusMessageKey[stage.status])}
                pulse={stage.status === 'current'}
                tone={toneByStage[stage.status]}
              />
            </div>
            <div>
              <h2>{t(stage.titleKey)}</h2>
              <p>{t(stage.detailKey)}</p>
            </div>
          </li>
        ))}
      </ol>

      <footer className="connection-rail__footer">
        <span>{t(transportKey)}</span>
        <span>{t(sessionKey)}</span>
      </footer>
    </aside>
  );
}
