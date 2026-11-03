import { Banner, Button, Details, Dialog, Field, Spinner } from '@agent-room/ui-system';
import { DoorOpen, LockKeyhole } from 'lucide-react';
import { useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  permissions,
  type CreatePrivateRoomInput,
  type PrivateRoom,
  type PrivateRoomFailure,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import { parseInvitationList } from '@/features/private-rooms/domain/private-room-invitations';
import { DEFAULT_INVITE_PERMISSIONS } from '@/features/private-rooms/domain/private-room-invite-link';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import {
  RoomListingChoices,
  type RoomListingChoice,
} from '@/features/private-rooms/ui/room-listing-choices';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { Result } from '@/shared/result';

const RETENTION_CHOICES = [7, 30, 90, 365] as const;

/**
 * 服务器说这份填写本身不行（400）：什么都没建，再发同一个请求也只会再被拒。放开表单让人改，
 * 下次是一个新请求（agent-room-ops#57）。
 */
const REJECTED_INPUT = new Set([
  'private_room.invalid_request',
  'private_room.invalid_creation_body',
  // 已经有一间公开房间：改成私密或者先去改另一间，再建。
  'private_room.public_limit_reached',
]);

/** 新建房间：连着服务，建好后直接进去。从发现页打开时一开始就选着公开。 */
export function NewRoomDialog({
  initiallyListed = false,
  onClose,
}: {
  readonly initiallyListed?: boolean;
  readonly onClose: () => void;
}) {
  const commands = useRoomCommands();
  return (
    <NewRoomForm initiallyListed={initiallyListed} onClose={onClose} onCreate={commands.create} />
  );
}

export type NewRoomFormProps = {
  readonly initiallyListed?: boolean;
  readonly onClose: () => void;
  /** 同一次填写的重试沿用同一个请求 ID，服务端不会再建一个房间。 */
  readonly onCreate: (
    requestId: string,
    input: CreatePrivateRoomInput,
  ) => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
};

/**
 * 一屏建好房间：名字、用途、私密还是公开（`specs/public-rooms/design.md`“网页”）；保留多久、现在就
 * 邀请谁放在“更多选项”里，不填就用默认值。选公开就说清楚不加密，新成员的权限和网络 Agent 先用默认的，
 * 以后在房间设置的“公开”里改。
 */
export function NewRoomForm({ initiallyListed = false, onClose, onCreate }: NewRoomFormProps) {
  const { t } = useTranslation();
  const formId = useId();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const [name, setName] = useState('');
  const [purpose, setPurpose] = useState('');
  const [listing, setListing] = useState<RoomListingChoice>({
    joinMode: 'open',
    listed: initiallyListed,
  });
  const [retention, setRetention] = useState('');
  const [inviteText, setInviteText] = useState('');
  const [invitePermissions, setInvitePermissions] = useState<PrivateRoomPermissions>(
    permissions('view', 'speak'),
  );
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const [failure, setFailure] = useState<string | null>(null);
  // 第一次提交后请求就定下来了：结果不明或进房间失败时，重试必须是同一个请求。
  const attempt = useRef<{ readonly id: string; readonly input: CreatePrivateRoomInput } | null>(
    null,
  );
  const locked = attempt.current !== null;
  const rejected = failure !== null && REJECTED_INPUT.has(failure);
  const invitations = parseInvitationList(inviteText);
  const inviteError = invitations.ok
    ? undefined
    : t(
        invitations.reason === 'too_many'
          ? 'rooms.create.invitesTooMany'
          : 'rooms.create.invitesInvalid',
      );
  const ready = name.trim().length > 0 && invitations.ok;

  const submit = async (): Promise<void> => {
    if (pendingRef.current || (!locked && !ready)) return;
    pendingRef.current = true;
    attempt.current ??= {
      id: identifiers.next(),
      input: {
        name: name.trim(),
        description: purpose.trim(),
        invitations: invitations.ok
          ? invitations.principalIds.map((principalId) => ({
              permissions: invitePermissions,
              principalId,
            }))
          : [],
        ...(retention === '' ? {} : { retentionDays: Number(retention) }),
        ...(listing.listed
          ? {
              listing: {
                allowNetworkAgents: true,
                joinMode: listing.joinMode,
                listed: true,
                memberPermissions: DEFAULT_INVITE_PERMISSIONS,
              },
            }
          : {}),
      },
    };
    setPending(true);
    setFailure(null);
    try {
      const result = await onCreate(attempt.current.id, attempt.current.input);
      if (result.ok) {
        onClose();
        return;
      }
      if (REJECTED_INPUT.has(result.error.code)) attempt.current = null;
      setFailure(result.error.code);
    } catch {
      setFailure('rooms.create_interrupted');
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };

  return (
    <Dialog
      className="new-room"
      closeLabel={t('rooms.close')}
      description={t('rooms.create.detail')}
      footer={
        <Button
          disabled={pending || (!locked && !ready)}
          form={formId}
          icon={pending ? <Spinner /> : <DoorOpen aria-hidden="true" />}
          type="submit"
        >
          {t(
            pending
              ? 'rooms.create.creating'
              : failure === null || rejected
                ? 'rooms.create.submit'
                : 'rooms.retry',
          )}
        </Button>
      }
      icon={<LockKeyhole />}
      onClose={() => {
        if (!pending) onClose();
      }}
      title={t('rooms.new')}
    >
      <form
        className="new-room__form"
        id={formId}
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <Field label={t('rooms.create.name')}>
          <input
            autoFocus
            disabled={pending || locked}
            maxLength={128}
            onChange={(event) => {
              setName(event.target.value);
            }}
            placeholder={t('rooms.create.namePlaceholder')}
            required
            value={name}
          />
        </Field>
        <Field label={t('rooms.create.purpose')}>
          <textarea
            disabled={pending || locked}
            maxLength={2_048}
            onChange={(event) => {
              setPurpose(event.target.value);
            }}
            placeholder={t('rooms.create.purposePlaceholder')}
            rows={3}
            value={purpose}
          />
        </Field>
        <RoomListingChoices
          disabled={pending || locked}
          name={formId}
          onChange={(next) => {
            setListing((current) => ({ ...current, ...next }));
          }}
          value={listing}
        />
        {listing.listed ? (
          <p className="room-listing__note room-listing__note--plain">
            {t('rooms.create.publicNote')}
          </p>
        ) : null}
        <Details className="new-room__more" summary={t('rooms.create.more')}>
          <Field label={t('rooms.create.retention')}>
            <select
              disabled={pending || locked}
              onChange={(event) => {
                setRetention(event.target.value);
              }}
              value={retention}
            >
              <option value="">{t('rooms.create.retentionDefault')}</option>
              {RETENTION_CHOICES.map((days) => (
                <option key={days} value={String(days)}>
                  {t('rooms.create.retentionDays', { count: days })}
                </option>
              ))}
            </select>
          </Field>
          <Field
            error={inviteError}
            hint={t('rooms.create.invitesHint')}
            label={t('rooms.create.invites')}
          >
            <textarea
              disabled={pending || locked}
              onChange={(event) => {
                setInviteText(event.target.value);
              }}
              placeholder={t('rooms.create.invitesPlaceholder')}
              rows={3}
              value={inviteText}
            />
          </Field>
          {invitations.ok && invitations.principalIds.length > 0 ? (
            <PrivateRoomCapabilityEditor
              disabled={pending || locked}
              legend={t('rooms.create.permissions')}
              onChange={setInvitePermissions}
              value={invitePermissions}
            />
          ) : null}
        </Details>
      </form>
      {failure === 'private_room.public_limit_reached' ? (
        <Banner tone="warning">{t('rooms.create.publicLimit')}</Banner>
      ) : failure === null ? null : (
        <Banner tone="danger" title={t(rejected ? 'rooms.create.rejected' : 'rooms.create.failed')}>
          <Details summary={t('rooms.details')}>
            <code>{failure}</code>
          </Details>
        </Banner>
      )}
    </Dialog>
  );
}
