import { useTranslation } from 'react-i18next';

import {
  privateRoomCapabilities,
  type PrivateRoomCapability,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';

export type PrivateRoomCapabilityEditorProps = {
  readonly disabled?: boolean;
  /**
   * 能加上的权限：没勾的、又不在这里面的勾不上（房主以外的人给不出自己没有的）。已经勾着的照样能去掉。
   * 不给就是都能加。
   */
  readonly grantable?: PrivateRoomPermissions;
  readonly legend: string;
  readonly onChange: (permissions: PrivateRoomPermissions) => void;
  readonly value: PrivateRoomPermissions;
};

export function PrivateRoomCapabilityEditor({
  disabled = false,
  grantable,
  legend,
  onChange,
  value,
}: PrivateRoomCapabilityEditorProps) {
  const { t } = useTranslation();
  const addable = (capability: PrivateRoomCapability): boolean =>
    grantable === undefined ||
    (grantable.capabilities.includes(capability) &&
      // 勾上自动发言会连带勾上发言。
      (capability !== 'automate' ||
        value.capabilities.includes('speak') ||
        grantable.capabilities.includes('speak')));
  return (
    <fieldset className="private-room-capabilities" disabled={disabled}>
      <legend>{legend}</legend>
      {privateRoomCapabilities.map((capability) => (
        <label key={capability}>
          <input
            checked={value.capabilities.includes(capability)}
            disabled={
              disabled ||
              capability === 'view' ||
              (!value.capabilities.includes(capability) && !addable(capability))
            }
            onChange={(event) => {
              onChange(toggleCapability(value, capability, event.target.checked));
            }}
            type="checkbox"
          />
          <span>
            <strong>{t(`privateRooms.capability.${capability}.label`)}</strong>
            <small>{t(`privateRooms.capability.${capability}.detail`)}</small>
          </span>
        </label>
      ))}
    </fieldset>
  );
}

export function toggleCapability(
  current: PrivateRoomPermissions,
  capability: PrivateRoomCapability,
  enabled: boolean,
): PrivateRoomPermissions {
  const next = new Set<PrivateRoomCapability>(current.capabilities);
  if (enabled) {
    next.add('view');
    next.add(capability);
    if (capability === 'automate') {
      next.add('speak');
    }
  } else if (capability !== 'view') {
    next.delete(capability);
    if (capability === 'speak') {
      next.delete('automate');
    }
  }
  return {
    capabilities: privateRoomCapabilities.filter((candidate) => next.has(candidate)),
  };
}
