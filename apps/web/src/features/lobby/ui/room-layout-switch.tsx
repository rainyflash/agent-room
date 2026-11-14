import { Segmented } from '@agent-room/ui-system';
import { Boxes, MessagesSquare } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { RoomPageLayout } from '@/features/lobby/domain/workspace-view';
import './room-layout-switch.css';

const icons = { conversation: MessagesSquare, scene: Boxes };

/**
 * 房间页顶栏上“对话 / 场景”的切换（`specs/redesign/room-page.md`）。两种版面的顶栏各放一个，
 * 换了以后新的那个接着拿焦点（见 `lobby-page.tsx`）。
 *
 * 手机上顶栏放不下两段，换成一个按钮：点一下去另一种版面。两样都在页面里，按宽度只显示一样。
 */
export function RoomLayoutSwitch({
  onChange,
  value,
}: {
  readonly onChange: (layout: RoomPageLayout) => void;
  readonly value: RoomPageLayout;
}) {
  const { t } = useTranslation();
  const other: RoomPageLayout = value === 'conversation' ? 'scene' : 'conversation';
  const OtherIcon = icons[other];
  const toOther = t(
    other === 'conversation'
      ? 'roomWorkspace.layout.toConversation'
      : 'roomWorkspace.layout.toScene',
  );
  return (
    <>
      <Segmented
        className="room-layout-switch"
        label={t('roomWorkspace.layout.label')}
        onChange={(next) => {
          if (next !== value) onChange(next);
        }}
        options={[
          {
            icon: <MessagesSquare />,
            label: t('roomWorkspace.layout.conversation'),
            value: 'conversation',
          },
          { icon: <Boxes />, label: t('roomWorkspace.layout.scene'), value: 'scene' },
        ]}
        value={value}
      />
      <button
        aria-label={toOther}
        className="room-layout-toggle ar-icon-button"
        onClick={() => {
          onChange(other);
        }}
        title={toOther}
        type="button"
      >
        <OtherIcon aria-hidden="true" />
      </button>
    </>
  );
}
