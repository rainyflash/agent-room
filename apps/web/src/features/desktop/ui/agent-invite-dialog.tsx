import { Button } from '@agent-room/ui-system';
import {
  AlertTriangle,
  Bot,
  Check,
  ChevronDown,
  CircleCheckBig,
  Copy,
  Download,
  PlugZap,
  RefreshCw,
  X,
} from 'lucide-react';
import { useEffect, useId, useLayoutEffect, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import { useOptionalSession } from '@/features/session/ui/session-provider';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import {
  agentInviteHosts,
  defaultInviteHost,
  inviteIdentityStorageKey,
  normalizeInviteDisplayName,
  projectInviteStatus,
  readInviteIdentity,
  writeInviteIdentity,
  type AgentInviteHost,
  type AgentInviteIdentity,
  type AgentInviteStatus,
} from '../domain/agent-invite';
import { desktopPhaseMessage } from '../domain/desktop-connection';
import type { AgentHostKind, HostSessionDiagnostics } from '../domain/desktop-runtime';
import { serializeManualHostConfiguration } from '../domain/manual-host-configuration';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import './agent-invite-dialog.css';

type InviteRoom = { readonly roomId: string; readonly roomName: string } | null;
type InviteOwner = { readonly principalId: string; readonly displayName: string } | null;

export type AgentInviteDialogProps = {
  /** 当前所在房间；没有房间上下文时 Agent 进入它的默认大厅。 */
  readonly room: InviteRoom;
  /** 已登录用户；缺省时从会话上下文读取。用于命名默认人物并记录身份归属。 */
  readonly owner?: InviteOwner;
  readonly downloadUrl: string | null;
  readonly onClose: () => void;
};

type InviteBodyProps = {
  readonly room: InviteRoom;
  readonly owner: InviteOwner;
  readonly downloadUrl: string | null;
  readonly onClose: () => void;
};

const hostLabels: Readonly<Record<Exclude<AgentInviteHost, 'other'>, string>> = {
  codex: 'Codex',
  'claude-code': 'Claude Code',
  cursor: 'Cursor',
};

const SESSION_POLL_MS = 3_000;
const SLOW_HINT_MS = 45_000;
const uuid = new BrowserUuidV7Factory();

type CopyState = 'idle' | 'copied' | 'failed';

export function AgentInviteDialog({
  room,
  owner = null,
  downloadUrl,
  onClose,
}: AgentInviteDialogProps) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);

  useLayoutEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    const trigger = document.activeElement;
    // jsdom 与个别旧内核没有 showModal；退化为普通 open 属性，仍保持可访问。
    if (typeof element.showModal === 'function') element.showModal();
    else element.setAttribute('open', '');
    return () => {
      if (typeof element.close === 'function') element.close();
      else element.removeAttribute('open');
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus();
    };
  }, []);

  // 挂到 body 上，避免继承「本机 Agent」面板等宿主容器的后代样式。
  return createPortal(
    <dialog
      aria-labelledby="agent-invite-title"
      className="agent-invite"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      ref={dialog}
    >
      <header className="agent-invite__header">
        <span className="agent-invite__mark">
          <Bot aria-hidden="true" />
        </span>
        <div>
          <h2 id="agent-invite-title">{t('agentInvite.title')}</h2>
          <p>
            {room === null
              ? t('agentInvite.subtitle.default')
              : t('agentInvite.subtitle.room', { room: room.roomName })}
          </p>
        </div>
        <button
          aria-label={t('agentInvite.close')}
          className="agent-invite__close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <InviteBody downloadUrl={downloadUrl} onClose={onClose} owner={owner} room={room} />
    </dialog>,
    document.body,
  );
}

function InviteBody({ room, owner, downloadUrl, onClose }: InviteBodyProps) {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  if (!controller.available) {
    return (
      <section className="agent-invite__web">
        <h3>{t('agentInvite.web.title')}</h3>
        <p>{t('agentInvite.web.description')}</p>
        {downloadUrl === null ? (
          <Button disabled icon={<Download aria-hidden="true" />} tone="quiet">
            {t('agentInvite.web.downloadPending')}
          </Button>
        ) : (
          <a className="ar-button ar-button--primary ar-button--default" href={downloadUrl}>
            <span className="ar-button__icon">
              <Download aria-hidden="true" />
            </span>
            <span>{t('agentInvite.web.download')}</span>
          </a>
        )}
      </section>
    );
  }
  return <DesktopInvite onClose={onClose} owner={owner} room={room} />;
}

function DesktopInvite({
  room,
  owner: ownerProp,
  onClose,
}: Pick<InviteBodyProps, 'room' | 'owner' | 'onClose'>) {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const session = useOptionalSession();
  const sessionPrincipal = session?.snapshot.context.principal ?? null;
  const owner =
    ownerProp ??
    (sessionPrincipal === null
      ? null
      : { principalId: sessionPrincipal.principalId, displayName: sessionPrincipal.displayName });
  const ownerId = owner?.principalId ?? null;
  const [chosenHost, setChosenHost] = useState<AgentInviteHost | null>(null);
  const host = chosenHost ?? defaultInviteHost(controller.hosts);
  const hostLabel = host === 'other' ? t('agentInvite.host.other') : hostLabels[host];

  const identities = useRef<Partial<Record<AgentInviteHost, AgentInviteIdentity>>>({});
  const [, bump] = useReducer((count: number) => count + 1, 0);
  const resolveIdentity = (target: AgentInviteHost): AgentInviteIdentity => {
    const cached = identities.current[target];
    if (cached !== undefined) return cached;
    const key = inviteIdentityStorageKey(target);
    const stored = readInviteIdentity(window.localStorage, key, ownerId);
    const label = target === 'other' ? t('agentInvite.host.other') : hostLabels[target];
    const created: AgentInviteIdentity = stored ?? {
      sessionKey: uuid.next(),
      displayName:
        owner === null
          ? t('agentInvite.defaultName.anonymous', { host: label })
          : t('agentInvite.defaultName', { host: label, owner: owner.displayName }),
      ownerId,
    };
    if (stored === null) writeInviteIdentity(window.localStorage, key, created);
    identities.current[target] = created;
    return created;
  };
  const identity = resolveIdentity(host);
  const replaceIdentity = (next: AgentInviteIdentity): void => {
    identities.current[host] = next;
    writeInviteIdentity(window.localStorage, inviteIdentityStorageKey(host), next);
    bump();
  };

  const [copyState, setCopyState] = useState<CopyState>('idle');
  const [copiedAt, setCopiedAt] = useState<number | null>(null);
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (copiedAt === null) return undefined;
    setSlow(false);
    const timer = setTimeout(() => {
      setSlow(true);
    }, SLOW_HINT_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [copiedAt]);

  const [sessions, setSessions] = useState<readonly HostSessionDiagnostics[] | null>(null);
  const [diagnosticsFailure, setDiagnosticsFailure] = useState<string | null>(null);
  const { readHostSessions } = controller;
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const result = await readHostSessions();
      if (disposed) return;
      if (result.ok) {
        setSessions(result.value);
        setDiagnosticsFailure(null);
      } else {
        setDiagnosticsFailure(result.error.code);
      }
      timer = setTimeout(() => void poll(), SESSION_POLL_MS);
    };
    void poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [readHostSessions]);
  const status: AgentInviteStatus =
    sessions === null ? { kind: 'waiting' } : projectInviteStatus(sessions, identity.sessionKey);

  const roomLine =
    room === null
      ? t('agentInvite.prompt.roomDefault')
      : t('agentInvite.prompt.roomKnown', { roomId: room.roomId, roomName: room.roomName });
  const prompt = t('agentInvite.prompt', {
    displayName: identity.displayName,
    room: roomLine,
    sessionKey: identity.sessionKey,
  });

  const copyPrompt = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(prompt);
      setCopyState('copied');
      setCopiedAt(Date.now());
    } catch {
      setCopyState('failed');
    }
  };

  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const detection = host === 'other' ? null : controller.hosts.find((entry) => entry.host === host);

  return (
    <div className="agent-invite__body">
      {phase === 'ready' ? null : (
        <p className="agent-invite__notice" role="status">
          <AlertTriangle aria-hidden="true" />
          {t('agentInvite.runtime.notReady', { phase: t(desktopPhaseMessage[phase]) })}
        </p>
      )}

      <ol className="agent-invite__steps">
        <li>
          <h3>{t('agentInvite.step.host')}</h3>
          <div
            className="agent-invite__hosts"
            role="radiogroup"
            aria-label={t('agentInvite.step.host')}
          >
            {agentInviteHosts.map((candidate) => {
              const found =
                candidate === 'other'
                  ? null
                  : controller.hosts.find((entry) => entry.host === candidate);
              return (
                <button
                  aria-checked={candidate === host}
                  className="agent-invite__host"
                  key={candidate}
                  onClick={() => {
                    setChosenHost(candidate);
                    setCopyState('idle');
                  }}
                  role="radio"
                  type="button"
                >
                  <strong>
                    {candidate === 'other' ? t('agentInvite.host.other') : hostLabels[candidate]}
                  </strong>
                  {candidate === 'other' ? null : (
                    <small data-installed={found?.installed === true ? 'true' : 'false'}>
                      {t(
                        found?.installed === true
                          ? 'agentInvite.host.installed'
                          : 'agentInvite.host.missing',
                      )}
                    </small>
                  )}
                </button>
              );
            })}
          </div>
          <HostSetup
            detection={detection}
            host={host}
            hostLabel={hostLabel}
            manualConfiguration={
              controller.snapshot === null
                ? null
                : serializeManualHostConfiguration(controller.snapshot.manualHostConfiguration)
            }
          />
        </li>

        <li>
          <h3>{t('agentInvite.step.copy')}</h3>
          <NameField
            key={identity.sessionKey}
            initial={identity.displayName}
            onCommit={(displayName) => {
              if (displayName !== identity.displayName) {
                replaceIdentity({ ...identity, displayName });
                setCopyState('idle');
              }
            }}
          />
          <Button
            className="agent-invite__copy"
            icon={
              copyState === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />
            }
            onClick={() => void copyPrompt()}
            size="large"
            tone={copyState === 'failed' ? 'alert' : 'primary'}
          >
            {copyState === 'copied' ? t('agentInvite.copied') : t('agentInvite.copy')}
          </Button>
          {copyState === 'failed' ? (
            <p className="agent-invite__error" role="status">
              <AlertTriangle aria-hidden="true" />
              {t('agentInvite.copyFailed')}
            </p>
          ) : null}
          <details className="agent-invite__preview" open={copyState === 'failed'}>
            <summary>
              <ChevronDown aria-hidden="true" />
              {t('agentInvite.preview')}
            </summary>
            <pre>{prompt}</pre>
          </details>
          <p className="agent-invite__note">
            {t('agentInvite.identityNote')}{' '}
            <button
              className="agent-invite__link"
              onClick={() => {
                replaceIdentity({ ...identity, sessionKey: uuid.next() });
                setCopyState('idle');
                setCopiedAt(null);
              }}
              type="button"
            >
              {t('agentInvite.newIdentity')}
            </button>
          </p>
        </li>

        <li>
          <h3>{t('agentInvite.step.wait')}</h3>
          <ArrivalStatus
            diagnosticsFailure={diagnosticsFailure}
            onDone={onClose}
            slow={slow && status.kind === 'waiting'}
            status={status}
          />
        </li>
      </ol>
    </div>
  );
}

function HostSetup({
  detection,
  host,
  hostLabel,
  manualConfiguration,
}: {
  readonly detection:
    | { readonly host: AgentHostKind; readonly installed: boolean; readonly configurable: boolean }
    | null
    | undefined;
  readonly host: AgentInviteHost;
  readonly hostLabel: string;
  readonly manualConfiguration: string | null;
}) {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const [jsonCopy, setJsonCopy] = useState<CopyState>('idle');
  const copyJson = async (): Promise<void> => {
    if (manualConfiguration === null) return;
    try {
      await navigator.clipboard.writeText(manualConfiguration);
      setJsonCopy('copied');
    } catch {
      setJsonCopy('failed');
    }
  };

  if (host === 'other' || (detection?.installed === true && !detection.configurable)) {
    return (
      <div className="agent-invite__setup">
        <p>
          {host === 'other'
            ? t('agentInvite.host.otherHint')
            : t('agentInvite.host.notConfigurable', { host: hostLabel })}
        </p>
        {manualConfiguration === null ? null : (
          <>
            <pre>{manualConfiguration}</pre>
            <Button
              icon={
                jsonCopy === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />
              }
              onClick={() => void copyJson()}
              size="compact"
              tone={jsonCopy === 'failed' ? 'alert' : 'ghost'}
            >
              {t(
                jsonCopy === 'copied' ? 'agentInvite.host.copiedJson' : 'agentInvite.host.copyJson',
              )}
            </Button>
          </>
        )}
      </div>
    );
  }

  if (detection?.installed !== true) {
    return (
      <div className="agent-invite__setup">
        <p>{t('agentInvite.host.missingHint', { host: hostLabel })}</p>
      </div>
    );
  }

  const configuring = controller.busy === 'host-configure';
  const configured = controller.configuredHost === host;
  return (
    <div className="agent-invite__setup">
      {configured ? (
        <p className="agent-invite__success" role="status">
          <CircleCheckBig aria-hidden="true" />
          {t('agentInvite.host.configured', { host: hostLabel })}
        </p>
      ) : (
        <Button
          disabled={controller.busy !== null}
          icon={configuring ? <RefreshCw aria-hidden="true" /> : <PlugZap aria-hidden="true" />}
          onClick={() => void controller.configureHost(host)}
          size="compact"
          tone="network"
        >
          {t(configuring ? 'agentInvite.host.configuring' : 'agentInvite.host.configure', {
            host: hostLabel,
          })}
        </Button>
      )}
      {controller.failure === null ? null : (
        <p className="agent-invite__error" role="alert">
          {t('agentInvite.host.failed', { code: controller.failure.code })}
        </p>
      )}
    </div>
  );
}

function NameField({
  initial,
  onCommit,
}: {
  readonly initial: string;
  readonly onCommit: (displayName: string) => void;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [draft, setDraft] = useState(initial);
  const normalized = normalizeInviteDisplayName(draft);
  return (
    <div className="agent-invite__name">
      <label htmlFor={id}>{t('agentInvite.name')}</label>
      <input
        aria-describedby={`${id}-hint`}
        aria-invalid={normalized === null}
        id={id}
        maxLength={128}
        onBlur={() => {
          if (normalized !== null) onCommit(normalized);
        }}
        onChange={(event) => {
          setDraft(event.target.value);
          const next = normalizeInviteDisplayName(event.target.value);
          if (next !== null) onCommit(next);
        }}
        type="text"
        value={draft}
      />
      <small className={normalized === null ? 'agent-invite__error' : undefined} id={`${id}-hint`}>
        {normalized === null ? t('agentInvite.name.invalid') : t('agentInvite.name.hint')}
      </small>
    </div>
  );
}

function ArrivalStatus({
  diagnosticsFailure,
  onDone,
  slow,
  status,
}: {
  readonly diagnosticsFailure: string | null;
  readonly onDone: () => void;
  readonly slow: boolean;
  readonly status: AgentInviteStatus;
}) {
  const { t } = useTranslation();
  if (diagnosticsFailure !== null && status.kind === 'waiting') {
    return (
      <p className="agent-invite__status agent-invite__error" data-kind="unavailable" role="status">
        <AlertTriangle aria-hidden="true" />
        {t('agentInvite.status.unavailable', { code: diagnosticsFailure })}
      </p>
    );
  }
  switch (status.kind) {
    case 'waiting':
      return (
        <div className="agent-invite__status" data-kind="waiting" role="status">
          <span aria-hidden="true" className="agent-invite__pulse" />
          <div>
            <strong>{t('agentInvite.status.waiting')}</strong>
            <p>{t(slow ? 'agentInvite.status.slow' : 'agentInvite.status.waitingHint')}</p>
          </div>
        </div>
      );
    case 'starting':
      return (
        <div className="agent-invite__status" data-kind="starting" role="status">
          <span aria-hidden="true" className="agent-invite__pulse" />
          <strong>{t('agentInvite.status.starting', { name: status.displayName })}</strong>
        </div>
      );
    case 'ready':
      return (
        <div className="agent-invite__status" data-kind="ready" role="status">
          <CircleCheckBig aria-hidden="true" />
          <div>
            <strong>{t('agentInvite.status.ready', { name: status.displayName })}</strong>
            <p>
              {t(status.active ? 'agentInvite.status.readyActive' : 'agentInvite.status.readyIdle')}
            </p>
          </div>
          <Button onClick={onDone} size="compact" tone="primary">
            {t('agentInvite.done')}
          </Button>
        </div>
      );
    case 'failed':
      return (
        <div className="agent-invite__status" data-kind="failed" role="alert">
          <AlertTriangle aria-hidden="true" />
          <div>
            <strong>{t('agentInvite.status.failed', { name: status.displayName })}</strong>
            {status.code === null ? null : <code>{status.code}</code>}
            <p>{t('agentInvite.status.failedHint')}</p>
          </div>
        </div>
      );
    case 'closed':
      return (
        <div className="agent-invite__status" data-kind="closed" role="status">
          <RefreshCw aria-hidden="true" />
          <strong>{t('agentInvite.status.closed', { name: status.displayName })}</strong>
        </div>
      );
  }
}
