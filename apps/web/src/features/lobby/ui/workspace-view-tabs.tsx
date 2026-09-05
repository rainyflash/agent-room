import { Boxes, Files, MessageCircle } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useId } from 'react';
import { useTranslation } from 'react-i18next';
import { roomWorkspaceViews, type RoomWorkspaceView } from '@/features/lobby/domain/workspace-view';

const viewIcons = { conversation: MessageCircle, resources: Files, space: Boxes };

export function WorkspaceViewTabs({
  value,
  onChange,
  allowSpace,
}: {
  readonly value: RoomWorkspaceView;
  readonly onChange: (value: RoomWorkspaceView) => void;
  readonly allowSpace: boolean;
}) {
  const { t } = useTranslation();
  const id = useId();
  const reduceMotion = useReducedMotion();
  const views = roomWorkspaceViews.filter((view) => allowSpace || view !== 'space');
  return (
    <div className="workspace-view-tabs" role="tablist" aria-label={t('roomWorkspace.views')}>
      {views.map((view, index) => {
        const Icon = viewIcons[view];
        return (
          <button
            aria-controls="workspace-current-view"
            aria-selected={value === view}
            id={`workspace-tab-${view}`}
            key={view}
            role="tab"
            tabIndex={value === view ? 0 : -1}
            type="button"
            onClick={() => {
              onChange(view);
            }}
            onKeyDown={(event) => {
              const offsets: Readonly<Record<string, number>> = { ArrowLeft: -1, ArrowRight: 1 };
              const next =
                event.key === 'Home'
                  ? 0
                  : event.key === 'End'
                    ? views.length - 1
                    : offsets[event.key] === undefined
                      ? null
                      : (index + (offsets[event.key] ?? 0) + views.length) % views.length;
              if (next === null || views[next] === undefined) return;
              event.preventDefault();
              onChange(views[next]);
              const tabs =
                event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
                  '[role="tab"]',
                );
              tabs?.[next]?.focus();
            }}
          >
            <Icon aria-hidden="true" />
            <span>{t(`roomWorkspace.${view}`)}</span>
            {value === view ? (
              <motion.span
                className="workspace-view-tabs__indicator"
                {...(reduceMotion === true ? {} : { layoutId: `view-${id}` })}
                transition={{ type: 'spring', stiffness: 420, damping: 34 }}
              />
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
