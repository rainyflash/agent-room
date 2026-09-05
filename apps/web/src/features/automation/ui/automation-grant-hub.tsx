import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Bot, CircleAlert, LoaderCircle, RefreshCw, X } from 'lucide-react';
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import {
  automationGrantListQueryKey,
  useAutomationGrantList,
} from '@/features/automation/data/automation-grant-queries';
import {
  isAutomationGrantActive,
  type AutomationGrantFailure,
  type AutomationGrantGateway,
  type CreateAutomationGrantInput,
} from '@/features/automation/domain/automation-grant';
import { AutomationGrantForm } from '@/features/automation/ui/automation-grant-form';
import { AutomationGrantList } from '@/features/automation/ui/automation-grant-list';
import { useAgentInstances } from '@/features/security/data/access-management-queries';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';

export type AutomationGrantHubProps = {
  readonly accessManagement: AccessManagementGateway;
  readonly automation: AutomationGrantGateway;
  readonly catalogId: string;
  readonly onReauthenticate: () => void;
  readonly recentlyAuthenticated: boolean;
  readonly roomName: string;
};

type AutomationCommand =
  | { readonly input: CreateAutomationGrantInput; readonly kind: 'create' }
  | { readonly grantId: string; readonly kind: 'revoke' };

export function AutomationGrantHub({
  accessManagement,
  automation,
  catalogId,
  onReauthenticate,
  recentlyAuthenticated,
  roomName,
}: AutomationGrantHubProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const reduceMotion = useReducedMotion();
  const grantsQuery = useAutomationGrantList(automation);
  const instancesQuery = useAgentInstances(accessManagement);
  const identifiers = useMemo(() => new BrowserUuidV7Factory(), []);
  const launcherRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const grants = grantsQuery.data?.ok === true ? grantsQuery.data.value : [];
  const roomGrants = grants.filter((grant) => grant.roomCatalogId === catalogId);
  const instances =
    instancesQuery.data?.ok === true
      ? instancesQuery.data.value.filter((instance) => instance.status !== 'revoked')
      : [];
  const activeCount = roomGrants.filter((grant) =>
    isAutomationGrantActive(grant, Date.now()),
  ).length;
  const mutation = useMutation({
    mutationFn: async (command: AutomationCommand) =>
      command.kind === 'create'
        ? await automation.create(identifiers.next(), command.input)
        : await automation.revoke(command.grantId),
    onSuccess: async (result) => {
      if (result.ok) {
        await queryClient.invalidateQueries({ queryKey: automationGrantListQueryKey });
      }
    },
  });
  const failure = mutation.data?.ok === false ? mutation.data.error : null;
  const listFailure = grantsQuery.data?.ok === false ? grantsQuery.data.error : null;
  const instancesFailure = instancesQuery.data?.ok === false ? instancesQuery.data.error : null;
  const pendingGrantId =
    mutation.isPending && mutation.variables.kind === 'revoke' ? mutation.variables.grantId : null;
  const successKind = mutation.data?.ok === true ? (mutation.variables?.kind ?? null) : null;

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    closeRef.current?.focus();
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !mutation.isPending) {
        setOpen(false);
        launcherRef.current?.focus();
      }
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => {
      window.removeEventListener('keydown', closeOnEscape);
    };
  }, [mutation.isPending, open]);

  const close = (): void => {
    if (mutation.isPending) {
      return;
    }
    setOpen(false);
    launcherRef.current?.focus();
  };

  const retry = async (): Promise<void> => {
    await Promise.all([grantsQuery.refetch(), instancesQuery.refetch()]);
  };

  return (
    <>
      <Button
        aria-label={t('automation.launcher')}
        className="automation-launcher"
        icon={<Bot aria-hidden="true" />}
        onClick={() => {
          mutation.reset();
          setOpen(true);
        }}
        ref={launcherRef}
        size="compact"
        tone="quiet"
      >
        {t('automation.launcher')}
        {activeCount === 0 ? null : (
          <span aria-label={t('automation.launcher.count', { count: activeCount })}>
            {activeCount}
          </span>
        )}
      </Button>
      {createPortal(
        <AnimatePresence>
          {open ? (
            <motion.div
              animate={{ opacity: 1 }}
              className="automation-overlay"
              exit={{ opacity: 0 }}
              initial={{ opacity: 0 }}
              key="automation-overlay"
              onMouseDown={(event) => {
                if (event.target === event.currentTarget) {
                  close();
                }
              }}
              transition={{ duration: reduceMotion ? 0 : 0.14 }}
            >
              <motion.aside
                animate={{ x: 0 }}
                aria-labelledby="automation-hub-title"
                aria-modal="true"
                className="automation-inspector"
                exit={{ x: '100%' }}
                initial={{ x: '100%' }}
                role="dialog"
                transition={
                  reduceMotion ? { duration: 0 } : { damping: 34, stiffness: 360, type: 'spring' }
                }
              >
                <header className="automation-inspector__topbar">
                  <div>
                    <p className="eyebrow">{t('automation.eyebrow')}</p>
                    <strong id="automation-hub-title">{t('automation.title')}</strong>
                    <p>{t('automation.detail')}</p>
                  </div>
                  <button
                    aria-label={t('automation.close')}
                    className="automation-close"
                    disabled={mutation.isPending}
                    onClick={close}
                    ref={closeRef}
                    type="button"
                  >
                    <X aria-hidden="true" />
                  </button>
                </header>

                <div className="automation-inspector__body">
                  <p aria-atomic="true" aria-live="polite" className="sr-only">
                    {successKind === null
                      ? ''
                      : t(
                          successKind === 'create'
                            ? 'automation.success.created'
                            : 'automation.success.revoked',
                        )}
                  </p>
                  {grantsQuery.isPending || instancesQuery.isPending ? (
                    <div className="automation-boundary" role="status">
                      <LoaderCircle aria-hidden="true" className="automation-spin" />
                      <strong>{t('automation.loading')}</strong>
                    </div>
                  ) : listFailure !== null || instancesFailure !== null ? (
                    <div className="automation-boundary" role="alert">
                      <CircleAlert aria-hidden="true" />
                      <div>
                        <strong>{t('automation.loadFailed')}</strong>
                        <code>{(listFailure ?? instancesFailure)?.code}</code>
                      </div>
                      <Button
                        icon={<RefreshCw aria-hidden="true" />}
                        onClick={() => void retry()}
                        size="compact"
                        tone="quiet"
                      >
                        {t('automation.retry')}
                      </Button>
                    </div>
                  ) : (
                    <>
                      {failure === null ? null : (
                        <AutomationFailure failure={failure} onReauthenticate={onReauthenticate} />
                      )}
                      <AutomationGrantForm
                        catalogId={catalogId}
                        instances={instances}
                        onCreate={(input) => {
                          mutation.mutate({ input, kind: 'create' });
                        }}
                        onReauthenticate={onReauthenticate}
                        pending={mutation.isPending}
                        recentlyAuthenticated={recentlyAuthenticated}
                        roomName={roomName}
                      />
                      <AutomationGrantList
                        grants={roomGrants}
                        instances={instances}
                        onReauthenticate={onReauthenticate}
                        onRevoke={(grantId) => {
                          mutation.mutate({ grantId, kind: 'revoke' });
                        }}
                        pendingGrantId={pendingGrantId}
                        recentlyAuthenticated={recentlyAuthenticated}
                      />
                    </>
                  )}
                </div>
              </motion.aside>
            </motion.div>
          ) : null}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}

function AutomationFailure({
  failure,
  onReauthenticate,
}: {
  readonly failure: AutomationGrantFailure;
  readonly onReauthenticate: () => void;
}) {
  const { t } = useTranslation();
  const reauthenticationRequired = failure.code === 'authentication.reauthentication_required';
  return (
    <p className="automation-inline-failure" role="alert">
      <CircleAlert aria-hidden="true" />
      <span>
        {t('automation.failure', { code: failure.code })}
        {failure.correlationId === undefined ? null : ` · ${failure.correlationId}`}
      </span>
      {reauthenticationRequired ? (
        <Button onClick={onReauthenticate} size="compact" tone="quiet">
          {t('automation.action.reauthenticate')}
        </Button>
      ) : null}
    </p>
  );
}
