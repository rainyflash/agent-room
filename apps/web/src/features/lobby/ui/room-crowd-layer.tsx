import { useImperativeHandle, useState, type Ref } from 'react';
import { useTranslation } from 'react-i18next';
import type { SceneFrame } from '../scene/scene-character';

export type RoomCrowdLayerHandle = { position(frame: SceneFrame): void };
const empty: NonNullable<SceneFrame['groups']> = [];

export function RoomCrowdLayer({
  ref,
  onFocus,
}: {
  readonly ref: Ref<RoomCrowdLayerHandle>;
  readonly onFocus: (x: number, y: number) => void;
}) {
  const { t } = useTranslation();
  const [groups, setGroups] = useState(empty);
  useImperativeHandle(ref, () => ({
    position: (frame) => {
      const next = frame.groups ?? empty;
      setGroups((previous) => (previous.length === 0 && next.length === 0 ? previous : next));
    },
  }));
  return (
    <div
      className="room-crowd-layer"
      role="group"
      aria-label={t('studio.crowd.label')}
      data-overview={groups.length > 0}
    >
      {groups.length === 0 ? null : <p className="room-crowd-hint">{t('studio.crowd.hint')}</p>}
      {groups.map((group) => (
        <button
          key={group.id}
          className="room-crowd-group"
          type="button"
          data-count={group.count}
          data-attention={group.attention > 0}
          data-activity={
            group.attention > group.count / 2
              ? 'attention'
              : group.working >= group.count / 2
                ? 'working'
                : 'available'
          }
          style={{ left: group.screenX, top: group.screenY }}
          aria-label={t('studio.crowd.open', {
            count: group.count,
            attention: group.attention,
            working: group.working,
          })}
          onClick={() => {
            onFocus(group.x, group.y);
          }}
        >
          <strong>{group.count}</strong>
          <span>{t('studio.crowd.agents')}</span>
          {group.attention > 0 ? (
            <small>{t('studio.crowd.attention', { count: group.attention })}</small>
          ) : null}
        </button>
      ))}
    </div>
  );
}
