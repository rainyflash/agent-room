import { Banner, Button, Field } from '@agent-room/ui-system';
import { useNavigate } from '@tanstack/react-router';
import { Download, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  ACCOUNT_DELETION_CONFIRMATION,
  REAUTHENTICATION_REQUIRED,
  type AccountGateway,
} from '@/features/account/domain/account';
import { rememberAccountDeletion } from '@/features/account/ui/account-deleted-notice';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import './account-settings.css';

const RETURN_PATH = '/settings/account';

/**
 * 账户：下载我的数据、删除账户（隐私说明里写的两条路）。删除要几分钟内登录过、输入 DELETE；
 * 成功后服务器已让这次登录失效，这里接着退出登录、清掉本机数据，回到首页说一声。
 */
export function AccountSettings() {
  const { t } = useTranslation();
  const { account } = useAppServices();
  const session = useOptionalSession();
  const principal = session?.snapshot.context.principal ?? null;
  if (account === undefined || session === null || principal === null) return null;
  return (
    <div className="settings-rows">
      <div className="settings-row">
        <div>
          <strong>{t('settings.account.signedInAs')}</strong>
          <p>{principal.displayName}</p>
        </div>
      </div>
      <ExportRow account={account} />
      <DeletionRow
        account={account}
        onDeleted={() => {
          session.send({ type: 'LOGOUT' });
        }}
        recentlyAuthenticated={principal.recentlyAuthenticated}
      />
    </div>
  );
}

function ExportRow({ account }: { readonly account: AccountGateway }) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const download = async (): Promise<void> => {
    setPending(true);
    setFailure(null);
    const exported = await account.exportData();
    setPending(false);
    if (!exported.ok) {
      setFailure(exported.error.code);
      return;
    }
    saveTextFile(exported.value.fileName, exported.value.json);
  };
  return (
    <div className="settings-row settings-row--stack">
      <div>
        <strong>{t('settings.account.export')}</strong>
        <p>{t('settings.account.exportHint')}</p>
        {failure === null ? null : (
          <p role="alert">{t('settings.account.exportFailed', { code: failure })}</p>
        )}
      </div>
      <Button
        disabled={pending}
        icon={<Download aria-hidden="true" />}
        onClick={() => {
          void download();
        }}
        size="compact"
        tone="ghost"
      >
        {t(pending ? 'settings.account.exportPending' : 'settings.account.exportAction')}
      </Button>
    </div>
  );
}

function DeletionRow({
  account,
  onDeleted,
  recentlyAuthenticated,
}: {
  readonly account: AccountGateway;
  readonly onDeleted: () => void;
  readonly recentlyAuthenticated: boolean;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  return (
    <div className="settings-row settings-row--stack">
      <div>
        <strong>{t('settings.account.delete')}</strong>
        <p>{t('settings.account.deleteHint')}</p>
      </div>
      {confirming ? (
        <DeletionConfirmation
          account={account}
          onCancel={() => {
            setConfirming(false);
          }}
          onDeleted={onDeleted}
          recentlyAuthenticated={recentlyAuthenticated}
        />
      ) : (
        <Button
          icon={<Trash2 aria-hidden="true" />}
          onClick={() => {
            setConfirming(true);
          }}
          size="compact"
          tone="ghost"
        >
          {t('settings.account.deleteOpen')}
        </Button>
      )}
    </div>
  );
}

function DeletionConfirmation({
  account,
  onCancel,
  onDeleted,
  recentlyAuthenticated,
}: {
  readonly account: AccountGateway;
  readonly onCancel: () => void;
  readonly onDeleted: () => void;
  readonly recentlyAuthenticated: boolean;
}) {
  const { t } = useTranslation();
  const { controlPlane } = useAppServices();
  const navigate = useNavigate();
  // 一次确认用同一个编号：网络断了重试，服务器只删一次。
  const idempotencyKey = useMemo(() => new BrowserUuidV7Factory().next(), []);
  const [acknowledged, setAcknowledged] = useState(false);
  const [typed, setTyped] = useState('');
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const needsSignIn = !recentlyAuthenticated || failure === REAUTHENTICATION_REQUIRED;
  const ready =
    acknowledged && typed.trim().toUpperCase() === ACCOUNT_DELETION_CONFIRMATION && !pending;
  const remove = async (): Promise<void> => {
    setPending(true);
    setFailure(null);
    const deleted = await account.requestDeletion(idempotencyKey);
    if (!deleted.ok) {
      setPending(false);
      setFailure(deleted.error.code);
      return;
    }
    rememberAccountDeletion();
    onDeleted();
    void navigate({ to: '/' });
  };
  return (
    <section aria-label={t('settings.account.delete')} className="account-deletion">
      <Banner icon={null} role={null} title={t('settings.account.deleteTitle')} tone="danger">
        <p>{t('settings.account.deleteRemoved')}</p>
        <p>{t('settings.account.deleteKept')}</p>
        {needsSignIn ? <p>{t('settings.account.deleteSignIn')}</p> : null}
      </Banner>
      {needsSignIn ? null : (
        <>
          <label className="account-deletion__acknowledge">
            <input
              checked={acknowledged}
              disabled={pending}
              onChange={(event) => {
                setAcknowledged(event.target.checked);
              }}
              type="checkbox"
            />
            <span>{t('settings.account.deleteAcknowledge')}</span>
          </label>
          <Field label={t('settings.account.deleteType', { word: ACCOUNT_DELETION_CONFIRMATION })}>
            <input
              autoComplete="off"
              disabled={pending}
              onChange={(event) => {
                setTyped(event.target.value);
              }}
              spellCheck={false}
              value={typed}
            />
          </Field>
        </>
      )}
      <div className="account-deletion__actions">
        {needsSignIn ? (
          <Button
            onClick={() => {
              void controlPlane.beginAuthentication(RETURN_PATH);
            }}
            size="compact"
            tone="primary"
          >
            {t('settings.account.deleteSignInAgain')}
          </Button>
        ) : (
          <Button
            disabled={!ready}
            onClick={() => {
              void remove();
            }}
            size="compact"
            tone="alert"
          >
            {t(pending ? 'settings.account.deletePending' : 'settings.account.deleteConfirm')}
          </Button>
        )}
        <Button disabled={pending} onClick={onCancel} size="compact" tone="quiet">
          {t('settings.account.deleteCancel')}
        </Button>
      </div>
      {failure !== null && failure !== REAUTHENTICATION_REQUIRED ? (
        <p role="alert">{t('settings.account.deleteFailed', { code: failure })}</p>
      ) : null}
    </section>
  );
}

function saveTextFile(fileName: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const anchor = document.createElement('a');
  anchor.download = fileName;
  anchor.href = url;
  anchor.rel = 'noopener';
  anchor.click();
  // 点击后浏览器才开始读这个地址，晚一点再释放。
  globalThis.setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 60_000);
}
