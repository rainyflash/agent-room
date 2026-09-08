import { memo } from 'react';
import { RoomPlan } from '../scene/svg/room-plan';
import { StudioSprite } from '../scene/svg/studio-sprite';

/** Decorative environment only; never contributes fabricated live participants. */
export const RoomIllustration = memo(function RoomIllustration() {
  return (
    <svg aria-hidden="true" className="room-illustration" viewBox="0 0 1536 1024">
      <RoomPlan world={{ width: 1536, height: 1024 }} />
    </svg>
  );
});

export function AgentPortrait({ id }: { readonly id: string }) {
  return (
    <svg aria-hidden="true" className="agent-portrait" focusable="false" viewBox="-50 -90 100 100">
      <StudioSprite id={id} portrait />
    </svg>
  );
}
