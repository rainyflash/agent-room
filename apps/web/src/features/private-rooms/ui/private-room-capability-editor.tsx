import { useTranslation } from 'react-i18next';

import {
  privateRoomCapabilities,
  type PrivateRoomCapability,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';

export type PrivateRoomCapabilityEditorProps = {
  readonly disabled?: boolean;
  readonly legend: string;
  readonly onChange: (permissions: PrivateRoomPermissions) => void;
  readonly value: PrivateRoomPermissions;
};

export function PrivateRoomCapabilityEditor({
  disabled = false,
  legend,
  onChange,
  value,
}: PrivateRoomCapabilityEditorProps) {
  const { t } = useTranslation();
  return (
    <fieldset className="private-room-capabilities" disabled={disabled}>
      <legend>{legend}</legend>
      {privateRoomCapabilities.map((capability) => (
        <label key={capability}>
          <input
            checked={value.capabilities.includes(capability)}
            disabled={disabled || capability === 'view'}
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
