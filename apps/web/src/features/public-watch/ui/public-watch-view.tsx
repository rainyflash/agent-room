import { Banner, Dialog } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Bot, MessageCircle, UsersRound, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { AppServicesProvider, useAppServices } from '@/app/app-services';
import { NetworkAgentInvite } from '@/features/desktop/ui/network-agent-invite';
import type { RoomLayout } from '@/features/lobby/domain/room-layout';
import { projectLobbyScene } from '@/features/lobby/domain/scene-projection';
import {
  LobbySpatialView,
  type LobbySpatialViewHandle,
} from '@/features/lobby/ui/lobby-spatial-view';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import type { PublicWatch } from '@/features/public-watch/domain/public-watch';
import { watchHumans, watchRoom, watchSpeech } from '@/features/public-watch/domain/watch-scene';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';
import { PublicWatchMessages, type WatchFocus } from './public-watch-messages';
import '@/features/lobby/ui/lobby-workspace.css';
import '@/features/lobby/ui/lobby-game.css';
import '@/features/desktop/ui/agent-invite-dialog.css';
import './public-watch.css';

// 计时接口要登录，和 FrontendTelemetryObserver 一样只记登录的人：围观页在登录范围外，场景计时不上报。
const silentTelemetry: FrontendTelemetryGateway = { record: () => Promise.resolve() };

/** 登录了没有：没问到之前什么都不显示，免得先闪一下登录按钮。 */
export type WatchAccount =
  | { readonly kind: 'unknown' }
  | { readonly kind: 'signed-in' }
  | {
      readonly kind: 'signed-out';
      readonly registrationOpen: boolean;
      readonly onSignIn: () => void;
      readonly onRegister: () => void;
    };

export type PublicWatchViewProps = {
  readonly watch: PublicWatch;
  /** 最近一次没读到：接着显示这份快照，同时说正在重试。 */
  readonly stale: boolean;
  readonly account: WatchAccount;
  /** 看的是默认大厅（`/watch`）：给 Agent 的那段话不写房间名，它进的就是默认大厅。 */
  readonly defaultLobby: boolean;
};

/**
 * 不登录也能看公共大厅（specs/public-lobby-watch/design.md）：和房间页同一套游戏大厅的样子，
 * 场景里是在线的 Agent 和最近说过话的人，右边是对话。只能看：点人物只跳到他说的话，没有输入框。
 */
export function PublicWatchView({ watch, stale, account, defaultLobby }: PublicWatchViewProps) {
  const { i18n, t } = useTranslation();
  const services = useAppServices();
  const sceneServices = useMemo(() => ({ ...services, telemetry: silentTelemetry }), [services]);
  const [view, setView] = useState<'conversation' | 'space'>('conversation');
  const [inviteOpen, setInviteOpen] = useState(false);
  const [focus, setFocus] = useState<WatchFocus | null>(null);
  const spatial = useRef<LobbySpatialViewHandle>(null);
  // 每次快照都接着上一次的站位摆：人物不会每 5 秒跳一次。
  const layout = useRef<RoomLayout | undefined>(undefined);
  const room = useMemo(() => watchRoom(watch), [watch]);
  const humans = useMemo(() => watchHumans(watch), [watch]);
  const projection = useMemo(
    () =>
      projectLobbyScene(room, null, {
        humans,
        ...(layout.current === undefined ? {} : { previous: layout.current }),
      }),
    [room, humans],
  );
  useEffect(() => {
    layout.current = projection.layout;
  }, [projection]);
  const speech = useMemo(() => watchSpeech(projection, watch), [projection, watch]);
  const updatedAt = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.resolvedLanguage ?? i18n.language, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      }).format(watch.updatedAtUnixMs),
    [i18n.language, i18n.resolvedLanguage, watch.updatedAtUnixMs],
  );
  // 只能看：点人物不弹详情，打开对话、跳到他最近说的那句。
  const showLatestFrom = (key: string): void => {
    setView('conversation');
    const latest = watch.messages.findLast((message) => message.author === key);
    if (latest !== undefined) setFocus({ key: latest.key });
  };
  const joinLink = (
    <Link
      className="ar-button ar-button--default ar-button--primary"
      params={{ catalogId: watch.lobby.catalogId }}
      search={{}}
      to="/lobby/$catalogId"
    >
      {t('publicWatch.join')}
      <ArrowRight aria-hidden="true" />
    </Link>
  );

  return (
    <main className="lobby-workspace lobby-game public-watch" id="main-content" data-view={view}>
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('lobby.liveSummary', { count: room.agents.length, room: watch.lobby.name })}
      </p>
      <div className="room-scene">
        <AppServicesProvider services={sceneServices}>
          <LobbySpatialView
            room={room}
            projection={projection}
            speech={speech}
            onOpenSpeech={(key) => {
              setView('conversation');
              setFocus({ key });
            }}
            onSelectHuman={showLatestFrom}
            selectedAgentId={null}
            onSelectAgent={(key) => {
              if (key !== null) showLatestFrom(key);
            }}
            ref={spatial}
          />
        </AppServicesProvider>
      </div>
      <header className="workspace-header public-watch__header">
        <a className="public-watch__home" href="/" aria-label={t('publicWatch.home')}>
          <img alt="" src="/agent-room-mark.svg" />
        </a>
        <div className="workspace-header__identity">
          <span>{t('publicWatch.eyebrow')}</span>
          <h1>{watch.lobby.name}</h1>
        </div>
        <p className="workspace-header__topic">{t('publicWatch.notice')}</p>
        <div className="public-watch__account">
          <div className="public-watch__language">
            <LanguageControl />
          </div>
          <AccountActions account={account} join={joinLink} />
        </div>
      </header>
      {room.agents.length === 0 ? (
        <div className="room-empty-presence" role="status">
          <strong>{t('publicWatch.noAgents.title')}</strong>
          <p>{t('publicWatch.noAgents.detail')}</p>
          <div className="room-empty-presence__actions">
            <button
              data-tone="primary"
              type="button"
              onClick={() => {
                setInviteOpen(true);
              }}
            >
              <Bot aria-hidden="true" />
              {t('studio.inviteAgent')}
            </button>
          </div>
        </div>
      ) : null}
      <div className="room-human-presence" role="group" aria-label={t('roomGame.people')}>
        {(projection.humans ?? []).slice(0, 3).map((human) => (
          <button
            type="button"
            key={human.matrixUserId}
            onClick={() => {
              showLatestFrom(human.matrixUserId);
            }}
            aria-label={t('roomGame.humanCharacter', { name: human.displayName })}
          >
            <UsersRound aria-hidden="true" />
            <span>{human.displayName}</span>
          </button>
        ))}
      </div>
      <nav className="room-toolbelt public-watch__toolbelt" aria-label={t('roomGame.actions')}>
        <button
          type="button"
          aria-pressed={view === 'conversation'}
          onClick={() => {
            setView(view === 'conversation' ? 'space' : 'conversation');
          }}
        >
          <MessageCircle aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.chat')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.chatShort')}
          </span>
        </button>
        <button
          type="button"
          aria-haspopup="dialog"
          aria-expanded={inviteOpen}
          onClick={() => {
            setInviteOpen(true);
          }}
        >
          <Bot aria-hidden="true" />
          <span className="room-toolbelt__label">{t('studio.inviteAgent')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.inviteShort')}
          </span>
        </button>
      </nav>
      <section
        className="room-panel public-watch__panel"
        hidden={view === 'space'}
        aria-label={t('roomGame.chat')}
      >
        <header className="room-panel__header">
          <h2 className="public-watch__panel-title">{t('roomGame.chat')}</h2>
          <div className="room-panel__header-actions">
            <button
              type="button"
              className="ar-icon-button"
              aria-label={t('roomGame.closePanel')}
              onClick={() => {
                setView('space');
                spatial.current?.focus();
              }}
            >
              <X aria-hidden="true" />
            </button>
          </div>
        </header>
        {stale ? (
          <Banner className="public-watch__stale" role="status" tone="warning">
            {t('publicWatch.stale', { time: updatedAt })}
          </Banner>
        ) : null}
        <div className="room-panel__content public-watch__content">
          <PublicWatchMessages focus={focus} watch={watch} />
        </div>
        <footer className="public-watch__footer">
          <p>
            <span className="public-watch__footer-notice">{t('publicWatch.notice')} </span>
            {t('publicWatch.readOnly')}
          </p>
          {joinLink}
        </footer>
      </section>
      {inviteOpen ? (
        <Dialog
          className="agent-invite"
          closeLabel={t('agentInvite.close')}
          description={t('publicWatch.invite.detail')}
          icon={<Bot />}
          onClose={() => {
            setInviteOpen(false);
          }}
          title={t('studio.inviteAgent')}
        >
          <NetworkAgentInvite
            room={
              defaultLobby ? null : { catalogId: watch.lobby.catalogId, roomName: watch.lobby.name }
            }
          />
        </Dialog>
      ) : null}
    </main>
  );
}

/** 右上角：没登录给登录和注册，登录完回到这个大厅；登录了直接进去说话。 */
function AccountActions({
  account,
  join,
}: {
  readonly account: WatchAccount;
  readonly join: ReactNode;
}) {
  const { t } = useTranslation();
  if (account.kind === 'unknown') return null;
  if (account.kind === 'signed-in') return join;
  return (
    <>
      <button
        className="ar-button ar-button--default ar-button--ghost public-watch__sign-in"
        onClick={account.onSignIn}
        type="button"
      >
        {t('landing.login')}
      </button>
      <button
        className="ar-button ar-button--default ar-button--primary"
        disabled={!account.registrationOpen}
        onClick={account.onRegister}
        type="button"
      >
        {t(account.registrationOpen ? 'landing.register' : 'landing.registrationPending')}
      </button>
    </>
  );
}
