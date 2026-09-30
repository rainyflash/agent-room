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
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import type { Result } from '@/shared/result';

const RETENTION_CHOICES = [7, 30, 90, 365] as const;

/** 新建房间：连着服务，建好后直接进去。 */
export function NewRoomDialog({ onClose }: { readonly onClose: () => void }) {
  const commands = useRoomCommands();
  return <NewRoomForm onClose={onClose} onCreate={commands.create} />;
}

export type NewRoomFormProps = {
  readonly onClose: () => void;
  /** 同一次填写的重试沿用同一个请求 ID，服务端不会再建一个房间。 */
  readonly onCreate: (
    requestId: string,
    input: CreatePrivateRoomInput,
  ) => Promise<Result<PrivateRoom, PrivateRoomFailure>>;
};

/**
 * 一屏建好房间：名字和用途；保留多久、现在就邀请谁放在“更多选项”里，不填就用默认值。
 * 能建的只有私人房间（公共大厅每种语言一个，由服务器开好），所以不让人选公开还是私人。
 */
export function NewRoomForm({ onClose, onCreate }: NewRoomFormProps) {
  const { t } = useTranslation();
  const formId = useId();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const [name, setName] = useState('');
  const [purpose, setPurpose] = useState('');
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
              : failure === null
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
      {failure === null ? null : (
        <Banner tone="danger" title={t('rooms.create.failed')}>
          <Details summary={t('rooms.details')}>
            <code>{failure}</code>
          </Details>
        </Banner>
      )}
    </Dialog>
  );
}
