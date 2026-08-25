import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import {
  AtSign,
  ChevronDown,
  ChevronUp,
  ListFilter,
  ListTodo,
  MessageCircle,
  MessageSquareText,
  PackageCheck,
  Pause,
  Play,
  Radio,
  RefreshCw,
  ShieldAlert,
  WifiOff,
  type LucideIcon,
} from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  orderSignalProjections,
  signalKinds,
  type SignalAction,
  type SignalKind,
  type SignalProjection,
} from '@/features/signals/domain/signal';
import { MessageProvenanceMark } from '@/features/messages/ui/message-provenance-mark';
import { formatDateTime, formatRelativeTime } from '@/shared/i18n/formatters';

const MAX_VISIBLE_SIGNALS = 50;

type SignalPresentation = {
  readonly icon: LucideIcon;
  readonly tone: StatusTone;
};

const presentationByKind: Readonly<Record<SignalKind, SignalPresentation>> = Object.freeze({
  direct_message: { icon: MessageCircle, tone: 'network' },
  handoff_pending: { icon: PackageCheck, tone: 'alert' },
  mention: { icon: AtSign, tone: 'alert' },
  room_message: { icon: MessageSquareText, tone: 'network' },
  sync_issue: { icon: WifiOff, tone: 'offline' },
  task_reference: { icon: ListTodo, tone: 'active' },
});

export type SignalDockProps = {
  readonly defaultExpanded?: boolean;
  readonly onAction: (action: SignalAction) => void;
  readonly onRetry: () => void;
  readonly selectedSignalId: string | null;
  readonly signals: readonly SignalProjection[];
  readonly state: 'failed' | 'ready';
};

export function SignalDock({
  defaultExpanded = false,
  onAction,
  onRetry,
  selectedSignalId,
  signals,
  state,
}: SignalDockProps) {
  const { i18n, t } = useTranslation();
  const orderedSignals = useMemo(() => orderSignalProjections(signals), [signals]);
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [filter, setFilter] = useState<SignalKind | 'all'>('all');
  const [frozen, setFrozen] = useState(false);
  const [frozenSignals, setFrozenSignals] = useState<readonly SignalProjection[]>([]);
  const displayedSignals = frozen ? frozenSignals : orderedSignals;
  const visibleSignals =
    filter === 'all'
      ? displayedSignals
      : displayedSignals.filter((signal) => signal.kind === filter);
  const availableKinds = signalKinds.filter((kind) =>
    displayedSignals.some((signal) => signal.kind === kind),
  );
  const featuredSignal = displayedSignals[0] ?? null;
  const language = i18n.resolvedLanguage;

  const toggleFrozen = (): void => {
    if (frozen) {
      setFrozen(false);
      return;
    }
    setFrozenSignals(orderedSignals);
    setFrozen(true);
  };

  return (
    <section
      aria-labelledby="signal-dock-title"
      className={`message-dock${expanded ? ' message-dock--expanded' : ''}`}
      data-frozen={frozen}
    >
      <header className="message-dock__header">
        <div className="message-dock__heading">
          <Radio aria-hidden="true" />
          <div>
            <p className="eyebrow">{t('signals.dock.eyebrow')}</p>
            <h2 id="signal-dock-title">{t('signals.dock.title')}</h2>
          </div>
        </div>
        <DockHeadline featuredSignal={featuredSignal} onAction={onAction} state={state} />
        <div className="message-dock__header-actions">
          <span aria-label={t('signals.dock.countLabel', { count: displayedSignals.length })}>
            {displayedSignals.length}
          </span>
          {state === 'failed' ? (
            <button aria-label={t('signals.dock.retry')} onClick={onRetry} type="button">
              <RefreshCw aria-hidden="true" />
            </button>
          ) : displayedSignals.length === 0 ? null : (
            <button
              aria-expanded={expanded}
              aria-label={t(expanded ? 'signals.dock.collapse' : 'signals.dock.expand')}
              onClick={() => {
                setExpanded((current) => !current);
              }}
              type="button"
            >
              {expanded ? <ChevronDown aria-hidden="true" /> : <ChevronUp aria-hidden="true" />}
            </button>
          )}
        </div>
      </header>
      {expanded && state === 'ready' ? (
        <>
          <div
            aria-label={t('signals.dock.controls')}
            className="message-dock__toolbar"
            role="toolbar"
          >
            <ListFilter aria-hidden="true" />
            <button
              aria-pressed={filter === 'all'}
              onClick={() => {
                setFilter('all');
              }}
              type="button"
            >
              {t('signals.filter.all')}
            </button>
            {availableKinds.map((kind) => (
              <button
                aria-pressed={filter === kind}
                key={kind}
                onClick={() => {
                  setFilter(kind);
                }}
                type="button"
              >
                {t(`signals.kind.${kind}`)}
              </button>
            ))}
            <button
              aria-pressed={frozen}
              className="message-dock__freeze"
              onClick={toggleFrozen}
              type="button"
            >
              {frozen ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
              {t(frozen ? 'signals.dock.resume' : 'signals.dock.freeze')}
            </button>
          </div>
          {visibleSignals.length === 0 ? (
            <div className="message-dock__empty">
              <MessageSquareText aria-hidden="true" />
              <p>{t('signals.dock.filteredEmpty')}</p>
            </div>
          ) : (
            <ol aria-label={t('signals.dock.listLabel')} className="message-dock__list">
              {visibleSignals.slice(0, MAX_VISIBLE_SIGNALS).map((signal) => (
                <SignalRow
                  language={language}
                  key={signal.signalId}
                  onAction={onAction}
                  selected={selectedSignalId === signal.signalId}
                  signal={signal}
                />
              ))}
            </ol>
          )}
        </>
      ) : null}
    </section>
  );
}

function DockHeadline({
  featuredSignal,
  onAction,
  state,
}: {
  readonly featuredSignal: SignalProjection | null;
  readonly onAction: (action: SignalAction) => void;
  readonly state: SignalDockProps['state'];
}) {
  const { t } = useTranslation();
  if (state === 'failed') {
    return (
      <div className="message-dock__headline message-dock__headline--failed">
        <ShieldAlert aria-hidden="true" />
        <span>{t('signals.dock.failed')}</span>
      </div>
    );
  }
  if (featuredSignal === null) {
    return <p className="message-dock__headline">{t('signals.dock.empty')}</p>;
  }
  const presentation = presentationByKind[featuredSignal.kind];
  const Icon = presentation.icon;
  return (
    <button
      className="message-dock__headline"
      onClick={() => {
        onAction(featuredSignal.action);
      }}
      type="button"
    >
      <Icon aria-hidden="true" />
      <span>{featuredSignal.title ?? t(`messages.lifecycle.${featuredSignal.lifecycle}`)}</span>
      {featuredSignal.actor === null ? null : (
        <MessageProvenanceMark provenance={featuredSignal.actor.provenance} />
      )}
    </button>
  );
}

function SignalRow({
  language,
  onAction,
  selected,
  signal,
}: {
  readonly language: string | undefined;
  readonly onAction: (action: SignalAction) => void;
  readonly selected: boolean;
  readonly signal: SignalProjection;
}) {
  const { t } = useTranslation();
  const presentation = presentationByKind[signal.kind];
  const Icon = presentation.icon;
  const actorName = signal.actor?.displayName ?? t(`signals.kind.${signal.kind}`);
  return (
    <li>
      <button
        aria-pressed={selected}
        className="message-signal"
        onClick={() => {
          onAction(signal.action);
        }}
        type="button"
      >
        <span aria-hidden="true" className="message-signal__author">
          {signal.actor === null ? <Icon /> : initials(signal.actor.displayName)}
        </span>
        <span className="message-signal__copy">
          <span className="message-signal__meta">
            <span className="message-signal__identity">
              <strong>{actorName}</strong>
              {signal.actor === null ? null : (
                <MessageProvenanceMark provenance={signal.actor.provenance} />
              )}
            </span>
            <time dateTime={new Date(signal.occurredAtUnixMs).toISOString()}>
              <span title={formatDateTime(signal.occurredAtUnixMs, language)}>
                {formatRelativeTime(signal.occurredAtUnixMs, Date.now(), language)}
              </span>
            </time>
          </span>
          <span className="message-signal__title">
            {signal.title ?? t(`messages.lifecycle.${signal.lifecycle}`)}
          </span>
          <span className="message-signal__summary">
            {signal.summary ?? t('messages.preview.unavailable')}
          </span>
          <span className="message-signal__footer">
            <StatusMark label={t(`signals.kind.${signal.kind}`)} tone={presentation.tone} />
            <span aria-hidden="true">{t(`signals.kind.${signal.kind}`)}</span>
            {signal.edited ? <span>{t('messages.preview.edited')}</span> : null}
            {signal.riskFlags.length === 0 ? null : (
              <span className="message-signal__risk">
                {t('messages.preview.riskCount', { count: signal.riskFlags.length })}
              </span>
            )}
          </span>
        </span>
      </button>
    </li>
  );
}

function initials(displayName: string): string {
  return [...displayName.trim()].slice(0, 2).join('').toUpperCase();
}
