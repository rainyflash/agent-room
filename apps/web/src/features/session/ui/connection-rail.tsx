import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { RadioTower } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { ConnectionStage } from '@/features/session/ui/connection-model';
import { LanguageControl } from '@/features/preferences/ui/language-control';

export type ConnectionRailProps = {
  readonly stages: readonly ConnectionStage[];
};

const toneByStage: Readonly<Record<ConnectionStage['status'], StatusTone>> = {
  blocked: 'alert',
  complete: 'active',
  current: 'network',
  pending: 'idle',
};

export function ConnectionRail({ stages }: ConnectionRailProps) {
  const { t } = useTranslation();
  return (
    <aside className="connection-rail">
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

      <ol aria-label={t('connection.progress')} className="connection-steps">
        {stages.map((stage) => (
          <li className={`connection-step connection-step--${stage.status}`} key={stage.titleKey}>
            <div className="connection-step__index">
              <span>{String(stage.index + 1).padStart(2, '0')}</span>
              <StatusMark
                label={t(`connection.stage.status.${stage.status}`)}
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
        <span>{t('connection.transport')}</span>
        <span>{t('connection.session')}</span>
      </footer>
    </aside>
  );
}
