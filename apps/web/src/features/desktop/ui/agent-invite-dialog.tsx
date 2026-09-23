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
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import { useOptionalSession } from '@/features/session/ui/session-provider';
import { BrowserUuidV7Factory } from '@/shared/ids/browser-uuid-v7-factory';
import {
  agentInviteHosts,
  defaultInviteHost,
  normalizeInviteDisplayName,
  projectInviteStatus,
  type AgentInviteHost,
  type AgentInviteIdentity,
  type AgentInviteStatus,
} from '../domain/agent-invite';
import {
  cliInvocation,
  encodeCliInvitation,
  readInviteHistory,
  saveInviteHistory,
} from '../domain/cli-invitation';
import { hostFailureMessage, localConnectionReady } from '../domain/desktop-connection';
import type { AgentHostKind, HostSessionDiagnostics } from '../domain/desktop-runtime';
import { serializeManualHostConfiguration } from '../domain/manual-host-configuration';
import { useDesktopRuntimeController } from './desktop-runtime-provider';
import { LocalConnectionNotice } from './local-connection-notice';
import './agent-invite-dialog.css';
import { useOptionalAppServices } from '@/app/app-services';
import { InviteReplyProgress } from './invite-reply-progress';
import type { ConnectedInvitation } from '../domain/invite-reply';

type InviteRoom = {
  readonly roomId: string;
  readonly roomName: string;
  readonly catalogId?: string;
} | null;
type InviteOwner = { readonly principalId: string; readonly displayName: string } | null;

export type AgentInviteDialogProps = {
  /** 当前所在房间；没有房间上下文时 Agent 进入它的默认大厅。 */
  readonly room: InviteRoom;
  /** 已登录用户；缺省时从会话上下文读取。用于命名默认人物并记录身份归属。 */
  readonly owner?: InviteOwner;
  readonly downloadUrl: string | null;
  readonly onClose: () => void;
  readonly onStartConversation?: (() => void) | undefined;
  readonly onConnected?: ((invitation: ConnectedInvitation) => void) | undefined;
};

type InviteBodyProps = {
  readonly room: InviteRoom;
  readonly owner: InviteOwner;
  readonly downloadUrl: string | null;
  readonly onClose: () => void;
  readonly onStartConversation?: (() => void) | undefined;
  readonly onConnected?: ((invitation: ConnectedInvitation) => void) | undefined;
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
  onStartConversation,
  onConnected,
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
          <p>{t('agentInvite.subtitle')}</p>
        </div>
        <button
          aria-label={t('agentInvite.close')}
          className="agent-invite__close ar-icon-button"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <InviteBody
        onConnected={onConnected}
        downloadUrl={downloadUrl}
        onClose={onClose}
        onStartConversation={onStartConversation}
        owner={owner}
        room={room}
      />
    </dialog>,
    document.body,
  );
}

function InviteBody({
  room,
  owner: ownerProp,
  downloadUrl,
  onClose,
  onStartConversation,
  onConnected,
}: InviteBodyProps) {
  const session = useOptionalSession();
  const principal = session?.snapshot.context.principal ?? null;
  const owner =
    ownerProp ??
    (principal === null
      ? null
      : {
          principalId: principal.principalId,
          displayName: principal.displayName,
        });
  return (
    <ConnectionInvite
      onConnected={onConnected}
      key={owner?.principalId ?? 'anonymous'}
      room={room}
      owner={owner}
      downloadUrl={downloadUrl}
      onClose={onClose}
      onStartConversation={onStartConversation}
    />
  );
}

function ConnectionInvite({
  room: currentRoom,
  owner,
  downloadUrl,
  onClose,
  onStartConversation,
  onConnected,
}: InviteBodyProps) {
  const { t } = useTranslation();
  const services = useOptionalAppServices();
  const controller = useDesktopRuntimeController();
  const ownerId = owner?.principalId ?? null;
  const [storage] = useState(() => {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  });
  const [mode, setMode] = useState<'cli' | 'mcp'>('cli');
  const [chosenHost, setChosenHost] = useState<AgentInviteHost | null>(null);
  const host = chosenHost ?? defaultInviteHost(controller.hosts);
  const hostLabel = host === 'other' ? t('agentInvite.host.other') : hostLabels[host];
  const makeIdentity = (): AgentInviteIdentity => ({
    sessionKey: uuid.next(),
    displayName:
      owner === null
        ? t('agentInvite.cli.anonymous')
        : t('agentInvite.cli.name', { owner: owner.displayName }),
    ownerId,
    room: currentRoom,
  });
  const [identity, setIdentity] = useState(makeIdentity);
  const room = identity.room === undefined ? currentRoom : identity.room;
  const [history, setHistory] = useState(() =>
    storage === null ? { identities: [], unavailable: true } : readInviteHistory(storage, ownerId),
  );
  const [restored, setRestored] = useState(false);
  const [validName, setValidName] = useState(true);
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const copyGeneration = useRef(0);
  useEffect(
    () => () => {
      copyGeneration.current += 1;
    },
    [],
  );
  const [copiedAt, setCopiedAt] = useState<number | null>(null);
  const [storageFailed, setStorageFailed] = useState(false);
  const [slow, setSlow] = useState(false);
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const localReady = localConnectionReady(phase);
  const cliConfiguration = controller.snapshot?.cliConfiguration ?? null;
  const detection = host === 'other' ? null : controller.hosts.find((entry) => entry.host === host);
  const { checkHost, readHostSessions } = controller;
  const setup = host === 'other' ? null : controller.hostSetup[host];
  // Claude Code is the only host with a skill folder today; check it whenever it is installed so the
  // invitation can shrink to one line as soon as the skill is current.
  const skillHost: AgentHostKind | null = controller.hosts.some(
    (entry) => entry.host === 'claude-code' && entry.installed,
  )
    ? 'claude-code'
    : null;
  const skillHostLabel = hostLabels['claude-code'];
  const { checkSkill } = controller;
  useEffect(() => {
    if (skillHost !== null) void checkSkill(skillHost);
  }, [skillHost, checkSkill]);
  const skillSetup = skillHost === null ? undefined : controller.skillSetup[skillHost];
  const skillCurrent = skillSetup?.phase === 'ready' && skillSetup.status.state === 'current';
  const canCopy =
    validName &&
    normalizeInviteDisplayName(identity.displayName) !== null &&
    (!controller.available ||
      (localReady &&
        (mode === 'cli'
          ? cliConfiguration !== null
          : host === 'other' || setup?.phase === 'configured')));

  useEffect(() => {
    if (
      mode === 'mcp' &&
      host !== 'other' &&
      detection?.installed === true &&
      detection.configurable
    )
      void checkHost(host);
  }, [mode, host, detection?.installed, detection?.configurable, checkHost]);

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
  useEffect(() => {
    if (!controller.available || !localReady) return undefined;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const result = await readHostSessions();
      if (disposed) return;
      if (result.ok) {
        setSessions(result.value);
        setDiagnosticsFailure(null);
      } else setDiagnosticsFailure(result.error.code);
      timer = setTimeout(() => void poll(), SESSION_POLL_MS);
    };
    void poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [controller.available, localReady, readHostSessions]);
  const status: AgentInviteStatus =
    sessions === null || !localReady
      ? { kind: 'waiting' }
      : projectInviteStatus(sessions, identity.sessionKey, room?.roomId);
  const invitedAgentId = sessions?.find((entry) => entry.sessionKey === identity.sessionKey)
    ?.session.agentId;
  const inviteRoomId = room?.roomId;
  useEffect(() => {
    if (
      status.kind !== 'ready' ||
      diagnosticsFailure !== null ||
      invitedAgentId == null ||
      inviteRoomId === undefined ||
      copiedAt === null
    )
      return;
    onConnected?.({
      agentId: invitedAgentId,
      roomId: inviteRoomId,
      startedAt: copiedAt,
      displayName: identity.displayName,
    });
  }, [
    status.kind,
    diagnosticsFailure,
    invitedAgentId,
    inviteRoomId,
    copiedAt,
    identity.displayName,
    onConnected,
  ]);
  const roomLine =
    room === null
      ? t('agentInvite.prompt.roomDefault')
      : t('agentInvite.prompt.roomKnown', { roomId: room.roomId, roomName: room.roomName });
  const invocation = cliInvocation(cliConfiguration, controller.snapshot?.platform ?? 'unknown');
  const scope = invocation + ' --profile ' + identity.sessionKey;
  const prompt =
    mode === 'cli'
      ? t(skillCurrent ? 'agentInvite.cli.promptWithSkill' : 'agentInvite.cli.prompt', {
          command:
            invocation +
            ' join --invite ' +
            encodeCliInvitation(identity, room?.roomId ?? null, room?.catalogId),
          scope,
          room: roomLine,
        })
      : t('agentInvite.prompt', {
          displayName: identity.displayName,
          room: roomLine,
          sessionKey: identity.sessionKey,
          target:
            room?.catalogId === undefined
              ? ''
              : '\n   room = ' + JSON.stringify({ catalogId: room.catalogId, roomId: room.roomId }),
        });

  const resetCopy = () => {
    copyGeneration.current += 1;
    setCopyState('idle');
    setCopiedAt(null);
  };
  // The instructions change shape once the skill is current, so an earlier copy is stale.
  const skillCurrentRef = useRef(skillCurrent);
  useEffect(() => {
    if (skillCurrentRef.current === skillCurrent) return;
    skillCurrentRef.current = skillCurrent;
    copyGeneration.current += 1;
    setCopyState('idle');
    setCopiedAt(null);
  }, [skillCurrent]);
  const newIdentity = () => {
    setIdentity(makeIdentity());
    setRestored(false);
    setValidName(true);
    setStorageFailed(false);
    resetCopy();
  };
  const copyPrompt = async (): Promise<void> => {
    if (!canCopy) return;
    const generation = ++copyGeneration.current;
    try {
      await navigator.clipboard.writeText(prompt);
      const saved = storage !== null && saveInviteHistory(storage, identity);
      if (generation !== copyGeneration.current) return;
      setCopyState('copied');
      setCopiedAt(Date.now());
      setStorageFailed(!saved);
      if (saved) setHistory(readInviteHistory(storage, ownerId));
    } catch {
      if (generation === copyGeneration.current) setCopyState('failed');
    }
  };
  return (
    <div className="agent-invite__body">
      {controller.available ? (
        <LocalConnectionNotice />
      ) : (
        <section className="agent-invite__web">
          <h3>{t('agentInvite.web.title')}</h3>
          <p>{t('agentInvite.web.description')}</p>
          {room?.catalogId ? (
            <a
              className="ar-button ar-button--primary ar-button--default"
              href={'agent-room://lobby/' + room.catalogId + '/instance/' + room.roomId}
            >
              {t('agentInvite.web.openDesktop')}
            </a>
          ) : null}
          {downloadUrl === null ? (
            <p>{t('agentInvite.web.downloadPending')}</p>
          ) : (
            <a className="ar-button ar-button--quiet ar-button--default" href={downloadUrl}>
              <Download aria-hidden="true" /> {t('agentInvite.web.download')}
            </a>
          )}
        </section>
      )}
      <ol className="agent-invite__steps">
        <li>
          <h3>{t('agentInvite.identity.title')}</h3>
          <p>{t('agentInvite.identity.description')}</p>
          {history.identities.length === 0 ? null : (
            <label className="agent-invite__restore">
              {t('agentInvite.identity.restore')}
              <select
                value={restored ? identity.sessionKey : ''}
                onChange={(event) => {
                  const previous = history.identities.find(
                    (entry) => entry.sessionKey === event.target.value,
                  );
                  if (previous === undefined) newIdentity();
                  else {
                    setIdentity(previous);
                    setRestored(true);
                    setValidName(true);
                    resetCopy();
                  }
                }}
              >
                <option value="">{t('agentInvite.identity.new')}</option>
                {history.identities.map((entry) => (
                  <option key={entry.sessionKey} value={entry.sessionKey}>
                    {entry.displayName} · {entry.sessionKey.slice(-6)}
                  </option>
                ))}
              </select>
            </label>
          )}
          <NameField
            key={identity.sessionKey}
            initial={identity.displayName}
            disabled={restored || copiedAt !== null}
            onValidityChange={setValidName}
            onCommit={(displayName) => {
              copyGeneration.current += 1;
              setIdentity((current) => ({ ...current, displayName }));
            }}
          />
          {restored && room?.roomId !== currentRoom?.roomId ? (
            <p role="status">
              {t('agentInvite.identity.restoredRoom', {
                room: room?.roomName ?? t('agentInvite.prompt.roomDefault'),
              })}
            </p>
          ) : null}
          <button className="agent-invite__link" type="button" onClick={newIdentity}>
            {t('agentInvite.identity.add')}
          </button>
          {history.unavailable || storageFailed ? (
            <p role="status" className="agent-invite__error">
              {t('agentInvite.identity.storageFailed')}
            </p>
          ) : null}
        </li>
        <li>
          <h3>{t('agentInvite.step.copy')}</h3>
          <p>{t('agentInvite.cli.description')}</p>
          {controller.available && cliConfiguration === null && mode === 'cli' ? (
            <p role="status">{t('agentInvite.cli.missing')}</p>
          ) : null}
          {mode === 'cli' && skillHost !== null ? (
            <SkillSetup host={skillHost} hostLabel={skillHostLabel} roomName={room?.roomName} />
          ) : null}
          <details className="agent-invite__advanced">
            <summary>{t('agentInvite.advanced')}</summary>
            <div
              className="agent-invite__modes"
              role="radiogroup"
              aria-label={t('agentInvite.mode')}
            >
              {(['cli', 'mcp'] as const).map((value) => (
                <button
                  key={value}
                  role="radio"
                  type="button"
                  aria-checked={mode === value}
                  onClick={() => {
                    setMode(value);
                    resetCopy();
                  }}
                >
                  {t(value === 'cli' ? 'agentInvite.mode.cli' : 'agentInvite.mode.mcp')}
                </button>
              ))}
            </div>
            {mode === 'mcp' ? (
              <>
                <p>{t('agentInvite.mcp.description')}</p>
                {controller.available ? (
                  <>
                    <div
                      className="agent-invite__hosts"
                      role="radiogroup"
                      aria-label={t('agentInvite.step.host')}
                    >
                      {agentInviteHosts.map((candidate) => (
                        <button
                          className="agent-invite__host"
                          key={candidate}
                          aria-checked={candidate === host}
                          role="radio"
                          type="button"
                          onClick={() => {
                            setChosenHost(candidate);
                            resetCopy();
                          }}
                        >
                          <strong>
                            {candidate === 'other'
                              ? t('agentInvite.host.other')
                              : hostLabels[candidate]}
                          </strong>
                          {candidate === 'cursor' ? (
                            <small>{t('agentInvite.host.foregroundOnly')}</small>
                          ) : null}
                        </button>
                      ))}
                    </div>
                    <HostSetup
                      detection={detection}
                      host={host}
                      hostLabel={hostLabel}
                      manualConfiguration={
                        controller.snapshot === null
                          ? null
                          : serializeManualHostConfiguration(
                              controller.snapshot.manualHostConfiguration,
                            )
                      }
                    />
                    {host === 'claude-code' && skillHost !== null ? (
                      <SkillSetup
                        host={skillHost}
                        hostLabel={skillHostLabel}
                        roomName={room?.roomName}
                      />
                    ) : null}
                  </>
                ) : (
                  <p>{t('agentInvite.mcp.web')}</p>
                )}
              </>
            ) : null}
            <p>{t('agentInvite.remote.description')}</p>
            <a
              href="https://github.com/rainyflash/agent-room/blob/main/infra/agent-runtime/README.md"
              target="_blank"
              rel="noreferrer"
            >
              {t('agentInvite.remote.docs')}
            </a>
          </details>
          <Button
            className="agent-invite__copy"
            disabled={!canCopy}
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
          <p className="agent-invite__note">{t('agentInvite.identityNote')}</p>
        </li>
        <li>
          <h3>{t('agentInvite.step.wait')}</h3>
          {controller.available ? (
            <ArrivalStatus
              preparation={canCopy ? (copiedAt === null ? 'instructions' : null) : 'setup'}
              diagnosticsFailure={diagnosticsFailure}
              onDone={onStartConversation ?? onClose}
              slow={slow && status.kind === 'waiting'}
              status={status}
            />
          ) : (
            <p role="status">{t('agentInvite.web.observe')}</p>
          )}
          {/* Once the agent is in the room, the paste instruction is stale; the first-reply step takes over. */}
          {copiedAt === null || status.kind === 'ready' ? null : (
            <p className="agent-invite__note">{t('agentInvite.progress.copied')}</p>
          )}
          {status.kind === 'ready' &&
          services !== null &&
          room !== null &&
          owner !== null &&
          invitedAgentId != null &&
          copiedAt !== null ? (
            <InviteReplyProgress
              gateway={services.messages}
              agentId={invitedAgentId}
              roomId={room.roomId}
              principalId={owner.principalId}
              startedAt={copiedAt}
            />
          ) : null}
          <p className="agent-invite__note">{t('agentInvite.receptionHint')}</p>
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

  const setup = controller.hostSetup[host];
  const configuring = setup?.phase === 'checking';
  const configured = setup?.phase === 'configured';
  return (
    <div className="agent-invite__setup">
      {configured ? (
        <p className="agent-invite__success" role="status">
          <CircleCheckBig aria-hidden="true" />
          {t('agentInvite.host.configured', { host: hostLabel })}
        </p>
      ) : (
        <Button
          disabled={controller.busy !== null || configuring}
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
      {setup?.phase !== 'failed' ? null : (
        <p className="agent-invite__error" role="alert">
          {t(hostFailureMessage(setup.error.code), { host: hostLabel })}
          <small>{t('agentInvite.errorCode', { code: setup.error.code })}</small>
        </p>
      )}
    </div>
  );
}

/** Installs this build's agent-room skill into the host; with it loaded, the invitation is one line. */
function SkillSetup({
  host,
  hostLabel,
  roomName,
}: {
  readonly host: AgentHostKind;
  readonly hostLabel: string;
  /** 当前房间；装好技能后提示用户以后直接说房间名，不必再复制。 */
  readonly roomName?: string | undefined;
}) {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const setup = controller.skillSetup[host];
  const status = setup?.phase === 'ready' ? setup.status : null;
  if (status?.state === 'unsupported') return null;
  const busy = setup?.phase === 'checking' || setup?.phase === 'installing';
  return (
    <div className="agent-invite__setup agent-invite__skill">
      {status?.state === 'current' ? (
        <>
          <p className="agent-invite__success" role="status">
            <CircleCheckBig aria-hidden="true" />
            {t('agentInvite.skill.current', { host: hostLabel })}
          </p>
          <p className="agent-invite__say">
            {t('agentInvite.skill.sayHint', { host: hostLabel })}{' '}
            <q>
              {roomName === undefined
                ? t('agentInvite.skill.sayLobby')
                : t('agentInvite.skill.sayRoom', { room: roomName })}
            </q>
          </p>
        </>
      ) : (
        <>
          <p>{t('agentInvite.skill.description', { host: hostLabel })}</p>
          <Button
            disabled={controller.busy !== null || busy}
            icon={busy ? <RefreshCw aria-hidden="true" /> : <PlugZap aria-hidden="true" />}
            onClick={() => void controller.installSkill(host)}
            size="compact"
            tone="network"
          >
            {t(
              setup?.phase === 'installing'
                ? 'agentInvite.skill.installing'
                : status?.state === 'outdated'
                  ? 'agentInvite.skill.update'
                  : 'agentInvite.skill.install',
              { host: hostLabel },
            )}
          </Button>
        </>
      )}
      {setup?.phase !== 'failed' ? null : (
        <p className="agent-invite__error" role="alert">
          {t('agentInvite.skill.failed', { host: hostLabel })}
          <small>{t('agentInvite.errorCode', { code: setup.error.code })}</small>
        </p>
      )}
    </div>
  );
}

function NameField({
  initial,
  onCommit,
  disabled = false,
  onValidityChange,
}: {
  readonly initial: string;
  readonly onCommit: (displayName: string) => void;
  readonly disabled?: boolean;
  readonly onValidityChange?: (valid: boolean) => void;
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
        disabled={disabled}
        id={id}
        maxLength={128}
        onBlur={() => {
          if (normalized !== null) onCommit(normalized);
        }}
        onChange={(event) => {
          setDraft(event.target.value);
          const next = normalizeInviteDisplayName(event.target.value);
          onValidityChange?.(next !== null);
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
  preparation,
  diagnosticsFailure,
  onDone,
  slow,
  status,
}: {
  readonly preparation: 'setup' | 'instructions' | null;
  readonly diagnosticsFailure: string | null;
  readonly onDone: () => void;
  readonly slow: boolean;
  readonly status: AgentInviteStatus;
}) {
  const { t } = useTranslation();
  if (diagnosticsFailure !== null) {
    return (
      <p className="agent-invite__status agent-invite__error" data-kind="unavailable" role="status">
        <AlertTriangle aria-hidden="true" />
        {t('agentInvite.status.unavailable', { code: diagnosticsFailure })}
      </p>
    );
  }
  if (status.kind === 'waiting' && preparation !== null) {
    return (
      <p className="agent-invite__status" data-kind="preparing" role="status">
        {t(
          preparation === 'setup'
            ? 'agentInvite.status.prepare'
            : 'agentInvite.status.instructions',
        )}
      </p>
    );
  }
  switch (status.kind) {
    case 'room_mismatch':
      return (
        <p className="agent-invite__status agent-invite__error" role="alert">
          {t('agentInvite.status.roomMismatch', { name: status.displayName })}
        </p>
      );
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
