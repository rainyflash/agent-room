import { Button } from '@agent-room/ui-system';
import { Focus, List, Minus, Plus, Radar } from 'lucide-react';
import { useTranslation } from 'react-i18next';

export type LobbyViewMode = 'list' | 'scene';

export type SignalDockProps = {
  readonly mode: LobbyViewMode;
  readonly onModeChange: (mode: LobbyViewMode) => void;
  readonly onResetViewport: () => void;
  readonly onZoomBy: (factor: number) => void;
  readonly sceneAvailable: boolean;
  readonly zoom: number;
};

export function SignalDock({
  mode,
  onModeChange,
  onResetViewport,
  onZoomBy,
  sceneAvailable,
  zoom,
}: SignalDockProps) {
  const { t } = useTranslation();
  return (
    <nav aria-label={t('lobby.dock.label')} className="signal-dock">
      <div className="signal-dock__modes">
        <Button
          aria-pressed={mode === 'scene'}
          disabled={!sceneAvailable}
          icon={<Radar aria-hidden="true" />}
          onClick={() => {
            onModeChange('scene');
          }}
          size="compact"
          tone={mode === 'scene' ? 'primary' : 'quiet'}
        >
          {t('lobby.dock.scene')}
        </Button>
        <Button
          aria-pressed={mode === 'list'}
          icon={<List aria-hidden="true" />}
          onClick={() => {
            onModeChange('list');
          }}
          size="compact"
          tone={mode === 'list' ? 'network' : 'quiet'}
        >
          {t('lobby.dock.list')}
        </Button>
      </div>
      {mode === 'scene' ? (
        <div className="signal-dock__zoom">
          <Button
            aria-label={t('lobby.dock.zoomOut')}
            icon={<Minus aria-hidden="true" />}
            onClick={() => {
              onZoomBy(0.84);
            }}
            size="compact"
            tone="quiet"
          >
            <span className="sr-only">{t('lobby.dock.zoomOut')}</span>
          </Button>
          <output aria-label={t('lobby.dock.zoom')}>{Math.round(zoom * 100)}%</output>
          <Button
            aria-label={t('lobby.dock.zoomIn')}
            icon={<Plus aria-hidden="true" />}
            onClick={() => {
              onZoomBy(1.19);
            }}
            size="compact"
            tone="quiet"
          >
            <span className="sr-only">{t('lobby.dock.zoomIn')}</span>
          </Button>
          <Button
            aria-label={t('lobby.dock.reset')}
            icon={<Focus aria-hidden="true" />}
            onClick={onResetViewport}
            size="compact"
            tone="quiet"
          >
            <span className="sr-only">{t('lobby.dock.reset')}</span>
          </Button>
        </div>
      ) : null}
      <div className="signal-dock__legend" aria-label={t('lobby.dock.legend')}>
        <span className="legend-signal legend-signal--active">{t('lobby.zone.active')}</span>
        <span className="legend-signal legend-signal--attention">{t('lobby.zone.attention')}</span>
        <span className="legend-signal legend-signal--available">{t('lobby.zone.available')}</span>
      </div>
    </nav>
  );
}
