import { useTranslation } from 'react-i18next';

import {
  roomJoinModes,
  type RoomJoinMode,
} from '@/features/private-rooms/domain/private-room-listing';

import './private-room-public-settings.css';

export type RoomListingChoice = {
  readonly joinMode: RoomJoinMode;
  readonly listed: boolean;
};

export type RoomListingChoicesProps = {
  readonly disabled: boolean;
  /** 同一页上两组单选不串：房间设置里用房间号，新建房间里用表单的 ID。 */
  readonly name: string;
  readonly onChange: (next: Partial<RoomListingChoice>) => void;
  readonly value: RoomListingChoice;
};

/**
 * 公开还是私密，公开了别人怎么进（`specs/public-rooms/design.md`“网页”）。房间设置的“公开”一节和
 * 新建房间共用；私密时不问怎么进。
 */
export function RoomListingChoices({ disabled, name, onChange, value }: RoomListingChoicesProps) {
  const { t } = useTranslation();
  return (
    <>
      <fieldset className="room-listing__choices" disabled={disabled}>
        <legend>{t('privateRooms.listing.visibility.legend')}</legend>
        {(['private', 'public'] as const).map((visibility) => (
          <label key={visibility}>
            <input
              checked={value.listed === (visibility === 'public')}
              name={`room-listing-visibility-${name}`}
              onChange={() => {
                onChange({ listed: visibility === 'public' });
              }}
              type="radio"
            />
            <span>
              <strong>{t(`privateRooms.listing.visibility.${visibility}.label`)}</strong>
              <small>{t(`privateRooms.listing.visibility.${visibility}.detail`)}</small>
            </span>
          </label>
        ))}
      </fieldset>
      {value.listed ? (
        <fieldset
          className="room-listing__choices room-listing__choices--three"
          disabled={disabled}
        >
          <legend>{t('privateRooms.listing.joinMode.legend')}</legend>
          {roomJoinModes.map((joinMode) => (
            <label key={joinMode}>
              <input
                checked={value.joinMode === joinMode}
                name={`room-listing-join-${name}`}
                onChange={() => {
                  onChange({ joinMode });
                }}
                type="radio"
              />
              <span>
                <strong>{t(`privateRooms.listing.joinMode.${joinMode}.label`)}</strong>
                <small>{t(`privateRooms.listing.joinMode.${joinMode}.detail`)}</small>
              </span>
            </label>
          ))}
        </fieldset>
      ) : null}
    </>
  );
}
