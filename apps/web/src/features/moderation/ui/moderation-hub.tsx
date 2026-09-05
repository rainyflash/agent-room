import { useOverlayContainer } from '@/shared/ui/overlay-container';
import { Button } from '@agent-room/ui-system';
import { CircleAlert, Gavel, LoaderCircle, RefreshCw, X } from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import {
  moderationActionListQueryKey,
  moderationAuditListQueryKey,
  moderationRoomCaseListQueryKey,
  useModerationActions,
  useModerationAudit,
  useModerationCapabilities,
  useModerationRoomCases,
} from '@/features/moderation/data/moderation-queries';
import type {
  ApplyModerationActionInput,
  ModerationFailure,
  ModerationGateway,
} from '@/features/moderation/domain/moderation';
import { ModerationActionForm } from '@/features/moderation/ui/moderation-action-form';
import {
  ModerationActionLedger,
  ModerationAuditLedger,
  ModerationCaseLedger,
} from '@/features/moderation/ui/moderation-ledger';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';

export type ModerationHubProps = {
  readonly catalogId: string;
  readonly gateway: ModerationGateway;
  readonly onReauthenticate: () => void;
  readonly recentlyAuthenticated: boolean;
  readonly roomName: string;
};

type ModerationCommand =
  | { readonly input: ApplyModerationActionInput; readonly kind: 'apply' }
  | { readonly actionId: string; readonly kind: 'reverse' };

export function ModerationHub({
  catalogId,
  gateway,
  onReauthenticate,
  recentlyAuthenticated,
  roomName,
}: ModerationHubProps) {
  const { t } = useTranslation();
  const overlayContainer = useOverlayContainer();
  const queryClient = useQueryClient();
  const reduceMotion = useReducedMotion();
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const launcherRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const capabilitiesQuery = useModerationCapabilities(gateway, catalogId);
  const capabilities = capabilitiesQuery.data?.ok === true ? capabilitiesQuery.data.value : null;
  const canModerateRoom = capabilities?.canModerateRoom === true;
  const canReadAudit = capabilities?.canReadAudit === true;
  const casesQuery = useModerationRoomCases(gateway, catalogId, canModerateRoom);
  const actionsQuery = useModerationActions(gateway, catalogId, canModerateRoom);
  const auditQuery = useModerationAudit(gateway, catalogId, canReadAudit);
  const cases = casesQuery.data?.ok === true ? casesQuery.data.value : null;
  const actions = actionsQuery.data?.ok === true ? actionsQuery.data.value : null;
  const audit = auditQuery.data?.ok === true ? auditQuery.data.value : null;
  const authorized = canModerateRoom || canReadAudit;
  const hasVisibleData = cases !== null || actions !== null || audit !== null;
  const mutation = useMutation({
    mutationFn: async (command: ModerationCommand) =>
      command.kind === 'apply'
        ? await gateway.applyAction(identifiers.next(), catalogId, command.input)
        : await gateway.reverseAction(command.actionId),
    onSuccess: async (result) => {
      if (!result.ok) {
        return;
      }
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: moderationActionListQueryKey(catalogId) }),
        queryClient.invalidateQueries({ queryKey: moderationRoomCaseListQueryKey(catalogId) }),
        queryClient.invalidateQueries({ queryKey: moderationAuditListQueryKey(catalogId) }),
      ]);
    },
  });
  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const pendingActionId =
    mutation.isPending && mutation.variables.kind === 'reverse'
      ? mutation.variables.actionId
      : null;

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    closeRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !mutation.isPending) {
        setOpen(false);
        launcherRef.current?.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [mutation.isPending, open]);

  if (!authorized) {
    return null;
  }

  const close = (): void => {
    if (mutation.isPending) {
      return;
    }
    setOpen(false);
    launcherRef.current?.focus();
  };
  const retry = async (): Promise<void> => {
    await Promise.all([
      capabilitiesQuery.refetch(),
      ...(canModerateRoom ? [casesQuery.refetch(), actionsQuery.refetch()] : []),
      ...(canReadAudit ? [auditQuery.refetch()] : []),
    ]);
  };

  return (
    <>
      <Button
        aria-label={t('moderation.governance.launcher')}
        icon={<Gavel aria-hidden="true" />}
        onClick={() => {
          mutation.reset();
          setOpen(true);
        }}
        ref={launcherRef}
        size="compact"
        tone="quiet"
      >
        {t('moderation.governance.launcher')}
      </Button>
      {createPortal(
        <AnimatePresence>
          {open ? (
            <motion.div
              animate={{ opacity: 1 }}
              className="moderation-governance-overlay"
              exit={{ opacity: 0 }}
              initial={{ opacity: 0 }}
              key="moderation-governance"
              onMouseDown={(event) => {
                if (event.target === event.currentTarget) {
                  close();
                }
              }}
              transition={{ duration: reduceMotion ? 0 : 0.14 }}
            >
              <motion.aside
                animate={{ x: 0 }}
                aria-labelledby="moderation-governance-title"
                aria-modal="true"
                className="moderation-governance"
                exit={{ x: '100%' }}
                initial={{ x: '100%' }}
                role="dialog"
                transition={
                  reduceMotion ? { duration: 0 } : { damping: 34, stiffness: 360, type: 'spring' }
                }
              >
                <header className="moderation-governance__header">
                  <div>
                    <p className="eyebrow">{t('moderation.governance.eyebrow')}</p>
                    <h2 id="moderation-governance-title">
                      {t('moderation.governance.title')} · {roomName}
                    </h2>
                    <p>{t('moderation.governance.detail')}</p>
                  </div>
                  <button
                    aria-label={t('moderation.governance.close')}
                    className="inspector-close"
                    disabled={mutation.isPending}
                    onClick={close}
                    ref={closeRef}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </header>
                <div className="moderation-governance__body">
                  {failure === null ? null : (
                    <ModerationFailureNotice
                      failure={failure}
                      onReauthenticate={onReauthenticate}
                    />
                  )}
                  {!hasVisibleData ? (
                    <ModerationBoundary onRetry={retry} />
                  ) : (
                    <div className="moderation-governance__workspace">
                      <div className="moderation-governance__primary">
                        {cases === null ? null : <ModerationCaseLedger cases={cases} />}
                        {actions === null ? null : (
                          <ModerationActionForm
                            cases={cases ?? []}
                            onApply={(input) => {
                              mutation.mutate({ input, kind: 'apply' });
                            }}
                            onReauthenticate={onReauthenticate}
                            pending={mutation.isPending}
                            recentlyAuthenticated={recentlyAuthenticated}
                          />
                        )}
                      </div>
                      <div className="moderation-governance__secondary">
                        {actions === null ? null : (
                          <ModerationActionLedger
                            actions={actions}
                            onReverse={(actionId) => {
                              mutation.mutate({ actionId, kind: 'reverse' });
                            }}
                            pendingActionId={pendingActionId}
                            recentlyAuthenticated={recentlyAuthenticated}
                          />
                        )}
                        {audit === null ? null : <ModerationAuditLedger events={audit} />}
                      </div>
                    </div>
                  )}
                </div>
              </motion.aside>
            </motion.div>
          ) : null}
        </AnimatePresence>,
        overlayContainer,
      )}
    </>
  );
}

function ModerationBoundary({ onRetry }: { readonly onRetry: () => Promise<void> }) {
  const { t } = useTranslation();
  return (
    <div className="moderation-boundary" role="alert">
      <CircleAlert aria-hidden="true" />
      <strong>{t('moderation.governance.failure', { code: 'moderation.unavailable' })}</strong>
      <Button
        icon={<RefreshCw aria-hidden="true" />}
        onClick={() => void onRetry()}
        size="compact"
        tone="quiet"
      >
        {t('moderation.governance.retry')}
      </Button>
    </div>
  );
}

function ModerationFailureNotice({
  failure,
  onReauthenticate,
}: {
  readonly failure: ModerationFailure;
  readonly onReauthenticate: () => void;
}) {
  const { t } = useTranslation();
  const reauthenticate = failure.code === 'authentication.reauthentication_required';
  return (
    <div className="moderation-inline-failure" role="alert">
      {failure.retryable ? <LoaderCircle aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}
      <span>{t('moderation.governance.failure', { code: failure.code })}</span>
      {reauthenticate ? (
        <Button onClick={onReauthenticate} size="compact" tone="quiet">
          {t('moderation.governance.action.reauthenticate')}
        </Button>
      ) : null}
    </div>
  );
}
