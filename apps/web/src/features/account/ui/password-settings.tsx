import { Banner, Button } from '@agent-room/ui-system';
import { useNavigate } from '@tanstack/react-router';
import { KeyRound, RotateCw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { AccountGateway } from '@/features/account/domain/account';
import type { SessionStateName } from '@/features/session/ui/connection-model';

/** 控制面在改好密码的回跳地址上加的标记（`specs/change-password/design.md`）。 */
export const PASSWORD_CHANGED_PARAMETER = 'passwordChanged';

/**
 * 看到标记以后先记在这个标签页里、从地址栏去掉：刷新不会再来一遍，等聊天连上的这段时间里页面
 * 跳走（比如去聊天登录）回来也还记得要退出别的地方。退成了才删。
 */
const PENDING_KEY = 'agent-room.password-changed.pending';

/** 这几种状态下聊天还在连，等它连上再退出别的地方，免得把这一台的聊天登录也注销了。 */
const CHAT_CONNECTING = new Set<SessionStateName>([
  'authenticating',
  'awaitingBrowserNavigation',
  'booting',
  'offline',
  'reconnecting',
  'restoring',
  'syncing',
]);

type FollowUp =
  | { readonly kind: 'idle' }
  | { readonly kind: 'waiting' }
  | { readonly kind: 'running' }
  | { readonly kind: 'done' }
  | { readonly code: string; readonly kind: 'failed' };

export type ChatConnection = {
  /** 这一台的聊天设备号；还没连上聊天时是 null。 */
  readonly deviceId: string | null;
  readonly state: SessionStateName;
};

/**
 * 设置“账户”里的“密码”：修改密码去身份服务改，改好回来接着退出别的浏览器和电脑上的登录。电脑授权和
 * Agent 连接不动，提示里说清楚。
 */
export function PasswordRow({
  account,
  beginChangePassword,
  chat,
  desktop,
}: {
  readonly account: AccountGateway;
  /** 去身份服务改密码；没去成时回错误码。 */
  readonly beginChangePassword: () => Promise<{ readonly code: string } | null>;
  readonly chat: ChatConnection;
  readonly desktop: boolean;
}) {
  const { t } = useTranslation();
  const followUp = usePasswordChangedFollowUp(account, chat);
  const [starting, setStarting] = useState(false);
  const [startFailure, setStartFailure] = useState<string | null>(null);
  const begin = async (): Promise<void> => {
    setStarting(true);
    setStartFailure(null);
    // 网页这就跳去身份服务；桌面端等人在系统浏览器里改完，回来整页重新打开。
    const failure = await beginChangePassword();
    setStarting(false);
    if (failure !== null) setStartFailure(failure.code);
  };
  return (
    <div className="settings-row settings-row--stack">
      <div>
        <strong>{t('settings.account.password')}</strong>
        <p>{t('settings.account.passwordHint')}</p>
        {starting && desktop ? (
          <p role="status">{t('settings.account.passwordInBrowser')}</p>
        ) : null}
        {startFailure === null ? null : (
          <p role="alert">{t('settings.account.passwordFailed', { code: startFailure })}</p>
        )}
      </div>
      <Button
        disabled={starting || followUp.state.kind === 'running'}
        icon={<KeyRound aria-hidden="true" />}
        onClick={() => {
          void begin();
        }}
        size="compact"
        tone="ghost"
      >
        {t(starting ? 'settings.account.passwordPending' : 'settings.account.passwordAction')}
      </Button>
      <PasswordChangedStatus followUp={followUp.state} onRetry={followUp.retry} />
    </div>
  );
}

function PasswordChangedStatus({
  followUp,
  onRetry,
}: {
  readonly followUp: FollowUp;
  readonly onRetry: () => void;
}) {
  const { t } = useTranslation();
  if (followUp.kind === 'idle') return null;
  if (followUp.kind === 'done') {
    return (
      <div className="account-password-status">
        <Banner title={t('settings.account.passwordChangedDone')} tone="success">
          <p>{t('settings.account.passwordChangedKept')}</p>
        </Banner>
      </div>
    );
  }
  if (followUp.kind === 'failed') {
    return (
      <div className="account-password-status">
        <Banner
          action={
            <Button
              icon={<RotateCw aria-hidden="true" />}
              onClick={onRetry}
              size="compact"
              tone="ghost"
            >
              {t('settings.account.passwordChangedRetry')}
            </Button>
          }
          title={t('settings.account.passwordChangedFailed')}
          tone="warning"
        >
          <p>{t('settings.account.passwordChangedFailedDetail', { code: followUp.code })}</p>
        </Banner>
      </div>
    );
  }
  return (
    <div className="account-password-status">
      <Banner title={t('settings.account.passwordChangedPending')} tone="info" />
    </div>
  );
}

function usePasswordChangedFollowUp(
  account: AccountGateway,
  chat: ChatConnection,
): { readonly retry: () => void; readonly state: FollowUp } {
  const navigate = useNavigate();
  const [fromAddress] = useState(passwordChangedInAddress);
  const [state, setState] = useState<FollowUp>(() =>
    fromAddress || readPending() ? { kind: 'waiting' } : { kind: 'idle' },
  );
  const inFlight = useRef(false);
  const chatSettled = chat.deviceId !== null || !CHAT_CONNECTING.has(chat.state);

  useEffect(() => {
    if (!fromAddress) return;
    writePending();
    void navigate({
      params: { section: 'account' },
      replace: true,
      search: {},
      to: '/settings/$section',
    });
  }, [fromAddress, navigate]);

  useEffect(() => {
    if (state.kind !== 'waiting' || !chatSettled || inFlight.current) return;
    inFlight.current = true;
    setState({ kind: 'running' });
    void account.signOutElsewhere(chat.deviceId).then((result) => {
      inFlight.current = false;
      if (result.ok) {
        clearPending();
        setState({ kind: 'done' });
      } else {
        setState({ code: result.error.code, kind: 'failed' });
      }
    });
  }, [account, chat.deviceId, chatSettled, state.kind]);

  return {
    retry: () => {
      setState({ kind: 'waiting' });
    },
    state,
  };
}

function passwordChangedInAddress(): boolean {
  try {
    return new URLSearchParams(window.location.search).get(PASSWORD_CHANGED_PARAMETER) === '1';
  } catch {
    return false;
  }
}

function readPending(): boolean {
  try {
    return window.sessionStorage.getItem(PENDING_KEY) === '1';
  } catch {
    return false;
  }
}

function writePending(): void {
  try {
    window.sessionStorage.setItem(PENDING_KEY, '1');
  } catch {
    // 存不了就只在这一页里记着：页面跳走以后就不接着退了，可以再改一次密码。
  }
}

function clearPending(): void {
  try {
    window.sessionStorage.removeItem(PENDING_KEY);
  } catch {
    // 同上。
  }
}
