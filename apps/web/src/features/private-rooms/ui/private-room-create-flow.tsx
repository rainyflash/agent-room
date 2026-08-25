import { Button } from '@agent-room/ui-system';
import { ArrowRight, Check, LoaderCircle, LockKeyhole, ShieldCheck } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  permissions,
  privateRoomPrincipalIdSchema,
  type CreatePrivateRoomInput,
  type PrivateRoomFailure,
  type PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import { PrivateRoomCapabilityEditor } from '@/features/private-rooms/ui/private-room-capability-editor';

export type PrivateRoomCreateFlowProps = {
  readonly failure: PrivateRoomFailure | null;
  readonly onCancel: () => void;
  readonly onCreate: (input: CreatePrivateRoomInput) => void;
  readonly pending: boolean;
};

type Details = {
  readonly description: string;
  readonly name: string;
  readonly retentionDays: number;
};

export function PrivateRoomCreateFlow({
  failure,
  onCancel,
  onCreate,
  pending,
}: PrivateRoomCreateFlowProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState(0);
  const [direction, setDirection] = useState(1);
  const [details, setDetails] = useState<Details>({
    description: '',
    name: '',
    retentionDays: 30,
  });
  const [inviteText, setInviteText] = useState('');
  const [invitePermissions, setInvitePermissions] = useState<PrivateRoomPermissions>(
    permissions('view', 'speak'),
  );
  const [validationKey, setValidationKey] = useState<string | null>(null);

  const move = (nextStep: number): void => {
    setDirection(nextStep > step ? 1 : -1);
    setStep(nextStep);
    setValidationKey(null);
  };

  const continueFromDetails = (): void => {
    if (details.name.trim().length === 0 || details.name.trim().length > 128) {
      setValidationKey('privateRooms.create.validation.name');
      return;
    }
    if (details.description.length > 2_048) {
      setValidationKey('privateRooms.create.validation.description');
      return;
    }
    move(1);
  };

  const continueFromInvites = (): void => {
    const parsed = parseInvitations(inviteText);
    if (!parsed.ok) {
      setValidationKey(parsed.errorKey);
      return;
    }
    move(2);
  };

  const submit = (): void => {
    const parsed = parseInvitations(inviteText);
    if (!parsed.ok) {
      move(1);
      setValidationKey(parsed.errorKey);
      return;
    }
    onCreate({
      description: details.description.trim(),
      invitations: parsed.principalIds.map((principalId) => ({
        permissions: invitePermissions,
        principalId,
      })),
      name: details.name.trim(),
      retentionDays: details.retentionDays,
    });
  };

  return (
    <section className="private-room-create" aria-labelledby="private-room-create-title">
      <header className="private-room-create__header">
        <div>
          <p className="eyebrow">{t('privateRooms.create.eyebrow')}</p>
          <h2 id="private-room-create-title">{t('privateRooms.create.title')}</h2>
        </div>
        <ol aria-label={t('privateRooms.create.progress')}>
          {[0, 1, 2].map((index) => (
            <li aria-current={index === step ? 'step' : undefined} key={index}>
              <span>{String(index + 1).padStart(2, '0')}</span>
              {t(`privateRooms.create.step${String(index + 1)}`)}
            </li>
          ))}
        </ol>
      </header>

      <div className="private-room-create__viewport">
        <AnimatePresence custom={direction} initial={false} mode="wait">
          <motion.div
            animate={{ opacity: 1, x: 0 }}
            className="private-room-create__step"
            exit={{ opacity: 0, x: direction * -28 }}
            initial={{ opacity: 0, x: direction * 28 }}
            key={step}
            transition={{ damping: 28, stiffness: 320, type: 'spring' }}
          >
            {step === 0 ? (
              <DetailsStep details={details} onChange={setDetails} />
            ) : step === 1 ? (
              <InvitationsStep
                invitePermissions={invitePermissions}
                inviteText={inviteText}
                onInvitePermissionsChange={setInvitePermissions}
                onInviteTextChange={setInviteText}
              />
            ) : (
              <SecurityStep inviteCount={parseInvitations(inviteText).principalIds.length} />
            )}
          </motion.div>
        </AnimatePresence>
      </div>

      {validationKey === null ? null : (
        <p className="private-room-feedback private-room-feedback--alert" role="alert">
          {t(validationKey)}
        </p>
      )}
      {failure === null ? null : <PrivateRoomFailureNotice failure={failure} />}

      <footer className="private-room-create__footer">
        <Button
          disabled={pending}
          onClick={step === 0 ? onCancel : () => move(step - 1)}
          size="compact"
          tone="quiet"
        >
          {step === 0 ? t('privateRooms.action.cancel') : t('privateRooms.action.back')}
        </Button>
        <Button
          disabled={pending}
          icon={
            pending ? (
              <LoaderCircle aria-hidden="true" className="private-room-spin" />
            ) : step === 2 ? (
              <Check aria-hidden="true" />
            ) : (
              <ArrowRight aria-hidden="true" />
            )
          }
          onClick={step === 0 ? continueFromDetails : step === 1 ? continueFromInvites : submit}
          size="default"
          tone="primary"
        >
          {pending
            ? t('privateRooms.create.creating')
            : step === 2
              ? t('privateRooms.create.submit')
              : t('privateRooms.action.continue')}
        </Button>
      </footer>
    </section>
  );
}

function DetailsStep({
  details,
  onChange,
}: {
  readonly details: Details;
  readonly onChange: (details: Details) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="private-room-form">
      <div className="private-room-step-heading">
        <span>01</span>
        <div>
          <h3>{t('privateRooms.create.details.title')}</h3>
          <p>{t('privateRooms.create.details.detail')}</p>
        </div>
      </div>
      <label className="private-room-field">
        <span>{t('privateRooms.create.name')}</span>
        <input
          autoFocus
          maxLength={128}
          onChange={(event) => onChange({ ...details, name: event.target.value })}
          placeholder={t('privateRooms.create.namePlaceholder')}
          value={details.name}
        />
        <small>{details.name.length}/128</small>
      </label>
      <label className="private-room-field">
        <span>{t('privateRooms.create.description')}</span>
        <textarea
          maxLength={2_048}
          onChange={(event) => onChange({ ...details, description: event.target.value })}
          placeholder={t('privateRooms.create.descriptionPlaceholder')}
          rows={4}
          value={details.description}
        />
        <small>{details.description.length}/2048</small>
      </label>
      <label className="private-room-field">
        <span>{t('privateRooms.create.retention')}</span>
        <select
          onChange={(event) => onChange({ ...details, retentionDays: Number(event.target.value) })}
          value={details.retentionDays}
        >
          {[7, 30, 90, 365].map((days) => (
            <option key={days} value={days}>
              {t('privateRooms.create.retentionDays', { count: days })}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}

function InvitationsStep({
  invitePermissions,
  inviteText,
  onInvitePermissionsChange,
  onInviteTextChange,
}: {
  readonly invitePermissions: PrivateRoomPermissions;
  readonly inviteText: string;
  readonly onInvitePermissionsChange: (permissions: PrivateRoomPermissions) => void;
  readonly onInviteTextChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="private-room-form">
      <div className="private-room-step-heading">
        <span>02</span>
        <div>
          <h3>{t('privateRooms.create.invites.title')}</h3>
          <p>{t('privateRooms.create.invites.detail')}</p>
        </div>
      </div>
      <label className="private-room-field">
        <span>{t('privateRooms.create.invites.principals')}</span>
        <textarea
          autoFocus
          onChange={(event) => onInviteTextChange(event.target.value)}
          placeholder={t('privateRooms.create.invites.placeholder')}
          rows={5}
          value={inviteText}
        />
        <small>{t('privateRooms.create.invites.hint')}</small>
      </label>
      <PrivateRoomCapabilityEditor
        legend={t('privateRooms.create.invites.permissions')}
        onChange={onInvitePermissionsChange}
        value={invitePermissions}
      />
    </div>
  );
}

function SecurityStep({ inviteCount }: { readonly inviteCount: number }) {
  const { t } = useTranslation();
  return (
    <div className="private-room-security">
      <div className="private-room-step-heading">
        <span>03</span>
        <div>
          <h3>{t('privateRooms.create.security.title')}</h3>
          <p>{t('privateRooms.create.security.detail')}</p>
        </div>
      </div>
      <article>
        <ShieldCheck aria-hidden="true" />
        <div>
          <strong>{t('privateRooms.create.security.inviteOnly')}</strong>
          <p>{t('privateRooms.create.security.inviteOnlyDetail', { count: inviteCount })}</p>
        </div>
        <span className="private-room-security__state private-room-security__state--active">
          {t('privateRooms.security.active')}
        </span>
      </article>
      <article>
        <LockKeyhole aria-hidden="true" />
        <div>
          <strong>{t('privateRooms.create.security.e2ee')}</strong>
          <p>{t('privateRooms.create.security.e2eeDetail')}</p>
        </div>
        <span className="private-room-security__state">{t('privateRooms.security.pending')}</span>
      </article>
      <p className="private-room-security__truth">{t('privateRooms.create.security.truth')}</p>
    </div>
  );
}

export function PrivateRoomFailureNotice({ failure }: { readonly failure: PrivateRoomFailure }) {
  const { t } = useTranslation();
  return (
    <div className="private-room-feedback private-room-feedback--alert" role="alert">
      <strong>{t('privateRooms.failure.title')}</strong>
      <span>{failure.code}</span>
      {failure.correlationId === undefined ? null : <code>{failure.correlationId}</code>}
    </div>
  );
}

type InvitationParseResult =
  | { readonly errorKey: string; readonly ok: false; readonly principalIds: readonly string[] }
  | { readonly ok: true; readonly principalIds: readonly string[] };

function parseInvitations(value: string): InvitationParseResult {
  const principalIds = Array.from(
    new Set(
      value
        .split(/[\s,;]+/u)
        .map((candidate) => candidate.trim())
        .filter(Boolean),
    ),
  );
  if (principalIds.length > 50) {
    return {
      errorKey: 'privateRooms.create.validation.inviteLimit',
      ok: false,
      principalIds,
    };
  }
  if (
    principalIds.some((candidate) => !privateRoomPrincipalIdSchema.safeParse(candidate).success)
  ) {
    return {
      errorKey: 'privateRooms.create.validation.principal',
      ok: false,
      principalIds,
    };
  }
  return { ok: true, principalIds };
}
