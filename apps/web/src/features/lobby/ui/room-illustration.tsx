import { memo } from 'react';
import { RoomPlan } from '../scene/svg/room-plan';
import { StudioSprite } from '../scene/svg/studio-sprite';

/** Marketing characters are decorative and never enter presence or directory data. */
export const RoomIllustration = memo(function RoomIllustration({
  populated = false,
}: {
  readonly populated?: boolean;
}) {
  return (
    <svg
      aria-hidden="true"
      className="room-illustration"
      viewBox={populated ? '0 0 1536 1280' : '0 0 1536 1024'}
    >
      <RoomPlan world={{ width: 1536, height: populated ? 1280 : 1024 }} />
      {populated ? (
        <g>
          <g transform="translate(490 470) scale(2.1)">
            <StudioSprite id="welcome-0" portrait />
          </g>
          <g transform="translate(1050 520) scale(2.1)">
            <StudioSprite id="welcome-1" portrait />
          </g>
          <g transform="translate(540 940) scale(2.1)">
            <StudioSprite id="welcome-2" portrait />
          </g>
          <g transform="translate(1100 990) scale(2.1)">
            <StudioSprite id="welcome-3" portrait />
          </g>
          <path d="M 764 644 L 813 672 L 813 728 L 764 756 L 715 728 L 715 672 Z" fill="#233039" />
          <circle cx="764" cy="700" r="18" fill="white" />
        </g>
      ) : null}
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
