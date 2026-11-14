import { Banner } from '@agent-room/ui-system';
import { ArrowDown, LockKeyhole, Search, SlidersHorizontal, UsersRound } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import {
  Fragment,
  useCallback,
  useContext,
  useDeferredValue,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import type {
  MessageRoomProjection,
  RoomNoticeChange,
  UndecryptableRecovery,
  UndecryptableSummary,
} from '@/features/messages/domain/message';
import { RoomNoticeChangeLine } from '@/features/room-notice/ui/room-notice-change';
import { agentRunFoldingEnabled, rememberAgentRunFolding } from '../application/agent-run-folding';
import { withNoticeChanges } from '../domain/conversation-entries';
import { foldAgentRuns, type ConversationItem } from '../domain/conversation-folds';
import { cursorOf, newMessagesAfter } from '../domain/new-messages';
import {
  conversationScopes,
  scopeConversation,
  unreadAddressed,
  type ConversationScope,
} from '../domain/conversation-scope';
import type { Result } from '@/shared/result';
import { emptyConversationFilter, searchConversation } from '../domain/conversation-search';
import { ConversationFoldRow } from './conversation-fold';
import { ExternalLinkGate } from './external-link';
import { ConversationSearch } from './conversation-search';
import { ConversationSearchRequestContext } from './conversation-search-request';
import { useConversationPosition } from './use-conversation-position';
import { useTranslation } from 'react-i18next';
import {
  conversationMessages,
  type ConversationParticipant,
} from '@/features/conversation/domain/conversation';
import { ConversationComposer } from '@/features/conversation/ui/conversation-composer';
import { ConversationMessage } from '@/features/conversation/ui/conversation-message';
import { useConversationComposer } from '@/features/conversation/ui/use-conversation-composer';
import type { MessageSubmissionIdFactory } from '@/features/messages/adapters/browser-submission-id-factory';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type { TypingNotifier } from '@/features/conversation/domain/typing';
import { mentionsAccount, type RoomMessageSignal } from '@/features/messages/domain/message';
import {
  compareCursors,
  type ReadingCursor,
} from '@/features/personal-workspace/domain/workspace-document';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { onWindowForegroundChange, windowInForeground } from '@/shared/browser/window-foreground';
import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';
import { useReceptionEvidence } from '@/features/desktop/ui/reception-evidence-context';
import { conversationDeliveries } from '../domain/message-delivery';
import './conversation-panel.css';

const emptyParticipants: readonly ConversationParticipant[] = [];
const emptyNoticeChanges: readonly RoomNoticeChange[] = [];
const noAcknowledged: ReadonlySet<string> = new Set();
// 进房间时本地通常只有同步带来的最近一小段记录。内容撑不满这么多屏、或者上次读到的位置还没加载，
// 就自动往前补；每次打开最多补这么多次，剩下的交给“加载更早的消息”。
const historyFillScreens = 1.5;
const maxAutomaticHistoryLoads = 3;

export type ConversationPanelProps = {
  readonly active?: boolean;
  readonly history?: MessageRoomProjection['history'];
  readonly onLoadOlder?: () => Promise<
    Result<void, { readonly code: string; readonly retryable: boolean }>
  >;
  readonly focusMessageId?: string | null;
  readonly onLatestDisplayed?: (matrixEventId: string) => void;
  readonly messages: readonly RoomMessageSignal[];
  readonly participants?: readonly ConversationParticipant[];
  /**
   * 房间成员叫什么，只用来写出被点名的人：Agent 点了还没说过话的人时靠它。不进 @ 菜单。
   * 优先级最低：说过话的照消息上的名字，Agent 照 `participants` 里的。
   */
  readonly memberNames?: readonly ConversationParticipant[];
  readonly publisher: MessagePublisher;
  readonly roomId: string;
  /** 打字时告诉房间里的人“正在输入”；不给就不说。 */
  readonly typing?: TypingNotifier | undefined;
  readonly writesAllowed?: boolean;
  readonly state: 'ready' | 'loading' | 'failed';
  readonly submissionIds?: MessageSubmissionIdFactory;
  /** 这台设备解不开的加密消息；有的话替代“还没有消息”，或者放在记录最前面。 */
  readonly undecryptable?: UndecryptableSummary;
  /** 找回这些消息走到了哪一步。 */
  readonly recovery?: UndecryptableRecovery;
  /** 公告改过几次，按时间夹在消息中间各写一行；查找时不显示。 */
  readonly noticeChanges?: readonly RoomNoticeChange[];
  readonly variant?: 'room' | 'direct';
  /** 私人房间：@ 菜单里多一项“所有人”。 */
  readonly privateRoom?: boolean;
  /** 顶上那一行右边、查找前面放的东西：对话版面里是“资料”。 */
  readonly barEnd?: ReactNode;
  /** 紧跟在顶上那一行下面的东西：对话版面里是公告和只读的提示。 */
  readonly belowBar?: ReactNode;
};

/**
 * 一段对话：顶上一行、时间线、输入框。正文里的地址点了先问一句，确认框（`ExternalLinkGate`）
 * 包在这一层：主房间、私聊、场景旁边那一块都带着。
 */
export function ConversationPanel(props: ConversationPanelProps) {
  return (
    <ExternalLinkGate>
      <ConversationPanelView {...props} />
    </ExternalLinkGate>
  );
}

function ConversationPanelView({
  active = true,
  history,
  onLoadOlder,
  focusMessageId: linkedMessageId = null,
  onLatestDisplayed,
  messages,
  participants = emptyParticipants,
  memberNames = emptyParticipants,
  publisher,
  roomId,
  typing,
  writesAllowed = true,
  state,
  submissionIds,
  undecryptable,
  recovery = 'idle',
  noticeChanges = emptyNoticeChanges,
  variant = 'room',
  privateRoom = false,
  barEnd,
  belowBar,
}: ConversationPanelProps) {
  const { t, i18n } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const receivers = useReceptionEvidence();
  const reduceMotion = useReducedMotion();
  const input = useRef<HTMLTextAreaElement>(null);
  const composer = useConversationComposer(publisher, roomId, submissionIds);
  const self = composer.publication.context.identity;
  const selfId = self?.matrixUserId ?? null;
  // 要去看的那一条：外面指着的（收件箱、查找、地址），或者人点了“回复的是哪句”要跳过去的。
  // 后点的算数；外面换了一条，跳过去的那条就作废。`turn` 让同一句点第二次也再跳一次。
  const [jump, setJump] = useState<{ readonly id: string; readonly turn: number } | null>(null);
  const [linked, setLinked] = useState(linkedMessageId);
  if (linked !== linkedMessageId) {
    setLinked(linkedMessageId);
    setJump(null);
  }
  const focusMessageId = jump?.id ?? linkedMessageId;
  const timeline = useMemo(() => conversationMessages(messages), [messages]);
  const messagesById = useMemo(
    () => new Map(messages.map((message) => [message.messageId, message])),
    [messages],
  );
  const [filter, setFilter] = useState(emptyConversationFilter);
  const deferredFilter = useDeferredValue(filter);
  // “只看”：全部、人说的、提到我的。私聊里只有两方，不分。
  const [scope, setScope] = useState<ConversationScope>('all');
  // 在“提到我的”里说了话：回到“全部”，不然刚说的那句自己看不到。只看最新的那一条，
  // 往前补出来的旧消息不算。
  const latestMessage = timeline.at(-1);
  const latestOwn = latestMessage?.actor.matrixUserId === selfId ? latestMessage.messageId : null;
  const [seenOwn, setSeenOwn] = useState(latestOwn);
  if (seenOwn !== latestOwn) {
    setSeenOwn(latestOwn);
    if (latestOwn !== null && scope === 'addressed') setScope('all');
    // 说了话就是回到了最新，跳过去看的那一句不再标着。
    if (latestOwn !== null) setJump(null);
  }
  const visibleTimeline = useMemo(
    () =>
      scopeConversation(searchConversation(timeline, deferredFilter), scope, selfId, messagesById),
    [timeline, deferredFilter, scope, selfId, messagesById],
  );
  const searching = Object.entries(filter).some(
    ([key, value]) => value !== (key === 'topic' ? null : ''),
  );
  // 眼前不是完整的对话时：不记读到哪儿、不算看过、不夹公告的改动、不折。
  const filtered = searching || scope !== 'all';
  const entries = useMemo(
    () => withNoticeChanges(visibleTimeline, filtered ? emptyNoticeChanges : noticeChanges),
    [visibleTimeline, filtered, noticeChanges],
  );
  // Agent 之间的长串折起来；关没关记在这台设备上。
  const [folding, setFolding] = useState(agentRunFoldingEnabled);
  const agentIds = useMemo(
    () =>
      new Set([
        ...participants.map((participant) => participant.matrixUserId),
        ...messages
          .filter((message) => message.actor.kind === 'agent')
          .map((message) => message.actor.matrixUserId),
      ]),
    [participants, messages],
  );
  const items = useMemo(
    (): readonly ConversationItem[] =>
      variant === 'room' && folding && !filtered
        ? foldAgentRuns(entries, { agents: agentIds, byId: messagesById })
        : entries,
    [variant, folding, filtered, entries, agentIds, messagesById],
  );
  // 哪几串打开着：人点开的，加上要去看的那条消息所在的那一串。后一种也记下来：
  // 之后不再指着那条消息了，正读着的这一串也不自己缩回去。每指一次只替人打开一次，
  // 人自己收起来的不再打开。
  const [openFolds, setOpenFolds] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const [openedFor, setOpenedFor] = useState<string | null>(null);
  const focusedFold =
    focusMessageId === null
      ? undefined
      : items.find(
          (item) =>
            item.kind === 'fold' &&
            item.messages.some((message) => message.messageId === focusMessageId),
        );
  if (focusMessageId === null) {
    if (openedFor !== null) setOpenedFor(null);
  } else if (focusedFold?.kind === 'fold' && openedFor !== focusMessageId) {
    setOpenedFor(focusMessageId);
    if (openFolds.get(focusedFold.key) !== true)
      setOpenFolds(new Map(openFolds).set(focusedFold.key, true));
  }
  const personal = usePersonalWorkspace();
  const workspace = personal?.snapshot.index;
  const readCursor = workspace?.read.get(roomId);
  const acknowledged = workspace?.acknowledged ?? noAcknowledged;
  const addressedUnread = useMemo(
    () =>
      variant === 'room'
        ? unreadAddressed(timeline, selfId, messagesById, { cursor: readCursor, acknowledged })
        : 0,
    [variant, timeline, selfId, messagesById, readCursor, acknowledged],
  );
  const scopeId = useId();
  // 要去看某一条消息时回到“全部”：它多半不在筛出来的那几条里。
  const [scopedFocus, setScopedFocus] = useState(focusMessageId);
  if (scopedFocus !== focusMessageId) {
    setScopedFocus(focusMessageId);
    if (focusMessageId !== null && scope !== 'all') setScope('all');
  }
  const [searchOpen, setSearchOpen] = useState(false);
  const searchId = useId();
  // 外面请求打开查找（手机上房间页“更多”里的那一项）：只有正在眼前的这一段对话应，
  // 打开以后焦点放进关键词那一格。一条消息都没有时没有查找，先告诉外面别摆那一项。
  const searchRequests = useContext(ConversationSearchRequestContext);
  const requestedTurn = searchRequests?.turn ?? 0;
  const [answeredTurn, setAnsweredTurn] = useState(requestedTurn);
  const [searchFocusTurn, setSearchFocusTurn] = useState(0);
  if (answeredTurn !== requestedTurn) {
    setAnsweredTurn(requestedTurn);
    if (active && timeline.length > 0) {
      setSearchOpen(true);
      setSearchFocusTurn(requestedTurn);
    }
  }
  useEffect(() => {
    if (searchFocusTurn === 0) return;
    document.getElementById(searchId)?.querySelector<HTMLElement>('input[type="search"]')?.focus();
  }, [searchFocusTurn, searchId]);
  const searchable = timeline.length > 0;
  const reportSearchable = searchRequests?.onAvailable;
  useEffect(() => {
    if (active) reportSearchable?.(searchable);
  }, [active, reportSearchable, searchable]);
  // 查看回复话题会直接打开查找区，关掉查找区就清除筛选。
  const searchVisible = searchOpen || filter.topic !== null;
  // 长消息默认收起；这里只记用户点过的，跳转定位到的那条和查找结果默认展开。
  const [expansions, setExpansions] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const [historyState, setHistoryState] = useState<'ready' | 'loading' | 'failed'>('ready');
  const historyLoading = useRef(false);
  const automaticLoads = useRef(0);
  // 在底部时自动补历史，补进来后要留在底部。中途往上翻了的话，这里记着他离底部的距离。
  const bottomDistance = useRef<number | null>(null);
  const timelineElement = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const position = useConversationPosition({
    roomId,
    timeline,
    element: timelineElement,
    following,
    active,
    filtered,
    focusMessageId,
  });
  const markRead = position.markRead;
  const [unseen, setUnseen] = useState(false);
  // “新消息”那条线画在哪儿：它后面别人说的算新的。人还没看到过最底下时，线跟着读到的位置走
  // （别的设备上读了、刚同步下来的也算）；看到底的那一刻定下来，之后读到的位置往前走，线留在
  // 原处，人看得出是从哪儿开始的。再来一批没看到的，挪到上一次看到底时最新的那一条后面；
  // 你说了话就撤掉。
  const [anchor, setAnchor] = useState<{ readonly after: ReadingCursor | null } | null>(null);
  const newAfter = anchor === null ? (readCursor ?? null) : anchor.after;
  // 读到的位置还在路上：账户没认出来，或者别的设备上读到哪儿还没同步下来。
  const cursorSettled =
    personal === null ||
    (personal.snapshot.accountId !== null && personal.snapshot.status !== 'loading');
  const cursorAtHand = useRef(readCursor);
  useLayoutEffect(() => {
    cursorAtHand.current = readCursor;
  });
  // 眼下这一批新消息看到底了没有；`seenThrough` 是上一次看到底时最新的那一条。
  const [caughtUp, setCaughtUp] = useState(false);
  const caughtUpNow = useRef(false);
  const seenThrough = useRef<ReadingCursor | null>(null);
  const fresh = useMemo(
    () => newMessagesAfter(timeline, selfId, newAfter),
    [timeline, selfId, newAfter],
  );
  // 正在筛、查找时不画线，按钮上也不写条数：数的是整个对话里的，眼前这几条里不一定有。
  const lineBefore = filtered ? null : fresh.firstId;
  const waiting = caughtUp || filtered ? 0 : fresh.count;
  const focusedMessage = useRef<HTMLDivElement>(null);
  const focusAvailable = visibleTimeline.some((message) => message.messageId === focusMessageId);
  const latestEvent = latestMessage?.matrixEventId;
  const displayedEvent = useRef<string | null>(null);
  // 人此刻看着最新的那一条：对话在眼前、窗口在前台、滚在最底下。看着才算看过、读到。
  const markLatestDisplayed = useCallback((): void => {
    const element = timelineElement.current;
    if (
      !active ||
      state !== 'ready' ||
      !windowInForeground() ||
      latestMessage === undefined ||
      latestEvent === undefined ||
      filtered ||
      position.missing ||
      !following.current ||
      element === null ||
      element.clientHeight === 0 ||
      element.scrollHeight - element.scrollTop - element.clientHeight >= 48
    )
      return;
    if (displayedEvent.current !== latestEvent) {
      displayedEvent.current = latestEvent;
      onLatestDisplayed?.(latestEvent);
    }
    seenThrough.current = cursorOf(latestMessage);
    if (!caughtUpNow.current) {
      caughtUpNow.current = true;
      setCaughtUp(true);
    }
    // 读到的位置还在路上时先不记“读到这儿了”：等它到了，线定在那以前读到的位置上，再往前挪。
    // 不等的话，这台设备刚打开就把别处同步下来的位置盖过去，线就没处画了。
    if (!cursorSettled) return;
    const before = cursorAtHand.current ?? null;
    setAnchor((current) => current ?? { after: before });
    markRead();
  }, [
    active,
    cursorSettled,
    latestMessage,
    latestEvent,
    onLatestDisplayed,
    state,
    filtered,
    position.missing,
    following,
    markRead,
  ]);
  useEffect(() => {
    const element = timelineElement.current;
    if (!active) return;
    // 外面指着某一条时不跟着新消息走；人自己跳过去看的那一句不拦，他滚回底下就接着跟。
    if (following.current && element !== null && linkedMessageId === null) {
      element.scrollTop = element.scrollHeight;
    } else if (latestEvent !== undefined) setUnseen(true);
  }, [active, latestEvent, linkedMessageId]);
  useEffect(() => {
    markLatestDisplayed();
    // 窗口回到前台、标签页重新看得见时再看一眼：不在前台那阵子来的话这时才算看到。
    return onWindowForegroundChange(markLatestDisplayed);
  }, [markLatestDisplayed]);
  // 来了新的一条：没看着的话，从上一次看到底的地方起算新的一批。要排在上面那个后面，
  // 看着的时候上面已经把它记成看过了。
  const arrived = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (latestMessage === undefined || arrived.current === latestMessage.messageId) return;
    const opening = arrived.current === undefined;
    arrived.current = latestMessage.messageId;
    // 打开时就在的：线照读到的位置画，不动。
    if (opening) return;
    if (latestMessage.actor.matrixUserId === selfId) {
      setAnchor({ after: null });
      return;
    }
    if (
      seenThrough.current !== null &&
      compareCursors(cursorOf(latestMessage), seenThrough.current) <= 0
    )
      return;
    if (caughtUpNow.current) {
      caughtUpNow.current = false;
      setCaughtUp(false);
      setAnchor({ after: seenThrough.current });
    }
  }, [latestMessage, selfId]);
  const jumpTurn = jump?.turn ?? 0;
  useEffect(() => {
    if (!active || focusMessageId === null || focusedMessage.current === null) return;
    focusedMessage.current.scrollIntoView({ block: 'center' });
    focusedMessage.current.focus({ preventScroll: true });
  }, [active, focusMessageId, focusAvailable, jumpTurn]);
  // 换了“只看”：从最新的那条看起。回到“全部”时也是，等于点了一次“回到最新”。
  const shownScope = useRef(scope);
  useLayoutEffect(() => {
    if (shownScope.current === scope) return;
    shownScope.current = scope;
    const element = timelineElement.current;
    if (element !== null && focusMessageId === null) element.scrollTop = element.scrollHeight;
  }, [scope, focusMessageId]);
  // 后面的盖住前面的：成员名单、你自己、消息上的发送者、房间里的 Agent。
  // 你自己和成员名单让 Agent 点了还没说过话的人时也写得出名字。
  const names = useMemo(
    () =>
      new Map([
        ...memberNames.map((member) => [member.matrixUserId, member.displayName] as const),
        ...(self === null ? [] : [[self.matrixUserId, self.displayName] as const]),
        ...messages.map(
          (message) => [message.actor.matrixUserId, message.actor.displayName] as const,
        ),
        ...participants.map(
          (participant) => [participant.matrixUserId, participant.displayName] as const,
        ),
      ]),
    [memberNames, self, messages, participants],
  );
  // 话题是回复关系连成的一串；只有参与了回复（回复过别人或被人回复）的消息才有话题可看，
  // 每条消息下都挂链接只是噪音。
  const topicMessageIds = useMemo(
    () =>
      new Set(
        messages.flatMap((message) =>
          message.relation === undefined
            ? []
            : [message.messageId, message.relation.targetMessageId],
        ),
      ),
    [messages],
  );
  const deliveries = useMemo(
    () => conversationDeliveries(timeline, receivers),
    [timeline, receivers],
  );
  const canEdit = writesAllowed && runtime.writes.allowed && composer.editable;
  const canSend = canEdit && state === 'ready' && composer.valid;
  const language = i18n.resolvedLanguage;
  const time = useMemo(
    () => new Intl.DateTimeFormat(language, { hour: '2-digit', minute: '2-digit' }),
    [language],
  );
  const date = useMemo(
    () => new Intl.DateTimeFormat(language, { month: 'long', day: 'numeric' }),
    [language],
  );
  const requestOlder = (automatic: boolean): void => {
    const element = timelineElement.current;
    if (onLoadOlder === undefined || historyLoading.current) return;
    historyLoading.current = true;
    if (automatic && following.current && element !== null)
      bottomDistance.current = element.scrollHeight - element.scrollTop;
    else position.preserve();
    setHistoryState('loading');
    const settle = (loaded: boolean): void => {
      historyLoading.current = false;
      bottomDistance.current = null;
      if (!loaded) position.release();
      setHistoryState(loaded ? 'ready' : 'failed');
    };
    void onLoadOlder().then(
      (result) => {
        settle(result.ok);
      },
      () => {
        settle(false);
      },
    );
  };
  const loadOlder =
    onLoadOlder === undefined
      ? undefined
      : (): void => {
          requestOlder(false);
        };
  // 正在往前补的时候先不说“找不到阅读位置”，补完还找不到再说。
  const positionMissing = position.missing && historyState !== 'loading';
  // 在绘制前决定：阅读位置还没加载时，先显示“正在加载”，而不是闪一下“找不到阅读位置”。
  useLayoutEffect(() => {
    const element = timelineElement.current;
    if (
      !active ||
      state !== 'ready' ||
      filtered ||
      onLoadOlder === undefined ||
      history?.canLoadMore !== true ||
      historyState !== 'ready' ||
      historyLoading.current ||
      automaticLoads.current >= maxAutomaticHistoryLoads ||
      element === null ||
      element.clientHeight === 0
    )
      return;
    if (!position.missing && element.scrollHeight >= element.clientHeight * historyFillScreens)
      return;
    automaticLoads.current += 1;
    // requestOlder 每次渲染都新建；它用到的状态都列在依赖里，其余是 ref。
    requestOlder(true);
  }, [active, state, filtered, onLoadOlder, history, historyState, position.missing, timeline]);
  // 前面补进更早的消息后，要在绘制前把跟随最新消息的人留在底部：WebKit 没有 CSS 滚动锚定，
  // 别的浏览器在内容刚变得能滚动、还停在顶端时也不锚定。自动补的过程中往上翻了的人，
  // 保持他离底部的距离，眼前的内容不跳。
  useLayoutEffect(() => {
    const element = timelineElement.current;
    if (!active || element === null || focusMessageId !== null) return;
    if (following.current) element.scrollTop = element.scrollHeight;
    else if (bottomDistance.current !== null)
      element.scrollTop = element.scrollHeight - bottomDistance.current;
  }, [active, focusMessageId, timeline, noticeChanges]);
  // 日期和“新消息”那两种线放在这条消息的框外面：标出要去看的那一条时只框住它自己。
  const renderMessage = (message: RoomMessageSignal, separator: ReactNode): ReactNode => (
    <Fragment key={message.messageId}>
      {separator}
      <div
        data-conversation-message-id={message.messageId}
        data-focused={message.messageId === focusMessageId}
        tabIndex={-1}
        ref={message.messageId === focusMessageId ? focusedMessage : undefined}
      >
        <ConversationMessage
          delivery={deliveries.get(message.messageId) ?? []}
          expanded={
            filter.text !== '' ||
            (expansions.get(message.messageId) ?? message.messageId === focusMessageId)
          }
          onExpandedChange={(expanded) => {
            setExpansions((current) => new Map(current).set(message.messageId, expanded));
          }}
          message={message}
          parent={
            message.relation === undefined
              ? undefined
              : messagesById.get(message.relation.targetMessageId)
          }
          names={names}
          time={time}
          editable={canEdit}
          own={message.actor.matrixUserId === selfId}
          mentioned={selfId !== null && mentionsAccount(message, selfId)}
          onReply={(target) => {
            composer.respond(target);
            input.current?.focus();
          }}
          onJump={(targetId) => {
            // 被回复的那句多半不在查找、筛出来的那几条里：都放开，回到完整的对话。
            setFilter(emptyConversationFilter);
            setSearchOpen(false);
            setScope('all');
            setJump((current) => ({ id: targetId, turn: (current?.turn ?? 0) + 1 }));
          }}
        />
        {topicMessageIds.has(message.messageId) ? (
          <button
            type="button"
            className="conversation-topic-link"
            onClick={() => {
              following.current = false;
              setFilter({ ...emptyConversationFilter, topic: message.messageId });
            }}
          >
            {t('history.viewTopic')}
          </button>
        ) : null}
      </div>
    </Fragment>
  );
  const newLine = (
    <div className="conversation-new" role="separator" aria-label={t('conversation.newLine')}>
      <span aria-hidden="true">{t('conversation.newLine')}</span>
    </div>
  );
  return (
    <section className="conversation-panel" aria-label={t('conversation.title')}>
      <h2 className="sr-only">{t('conversation.title')}</h2>
      <div className="conversation-panel__bar">
        {variant === 'direct' ? (
          <span className="conversation-panel__context">
            <UsersRound aria-hidden="true" />
            <span>{t('roomWorkspace.privateHint')}</span>
          </span>
        ) : (
          <div
            className="conversation-panel__scopes"
            role="group"
            aria-label={t('conversation.scope.label')}
          >
            {conversationScopes.map((value) => {
              const unread = value === 'addressed' && addressedUnread > 0;
              return (
                <button
                  type="button"
                  key={value}
                  // 名字不变，没读的条数经 aria-describedby 读出来。
                  aria-label={unread ? t(`conversation.scope.${value}`) : undefined}
                  aria-describedby={unread ? scopeId : undefined}
                  aria-pressed={scope === value}
                  onClick={() => {
                    if (value === scope) return;
                    setScope(value);
                    setJump(null);
                    position.latest();
                    setUnseen(false);
                  }}
                >
                  {t(`conversation.scope.${value}`)}
                  {unread ? (
                    <span className="conversation-panel__scope-count" id={scopeId}>
                      <span aria-hidden="true">
                        {addressedUnread > 99 ? '99+' : addressedUnread}
                      </span>
                      <span className="sr-only">
                        {t('conversation.scope.unread', { count: addressedUnread })}
                      </span>
                    </span>
                  ) : null}
                </button>
              );
            })}
          </div>
        )}
        {barEnd}
        {/* 还没有任何消息时，查找只是噪音。 */}
        {timeline.length === 0 ? null : (
          <button
            type="button"
            className="conversation-panel__bar-button conversation-panel__bar-button--icon"
            aria-expanded={searchVisible}
            aria-controls={searchId}
            aria-label={t('history.search')}
            title={t('history.search')}
            onClick={() => {
              if (searchVisible) {
                setSearchOpen(false);
                setFilter(emptyConversationFilter);
              } else setSearchOpen(true);
            }}
          >
            <Search aria-hidden="true" />
          </button>
        )}
        <details className="conversation-panel__availability">
          <summary aria-label={t('conversation.details')} title={t('conversation.details')}>
            <SlidersHorizontal aria-hidden="true" />
          </summary>
          <div>
            {variant === 'room' ? (
              <label className="conversation-panel__option">
                <input
                  type="checkbox"
                  checked={folding}
                  onChange={(event) => {
                    setFolding(event.target.checked);
                    rememberAgentRunFolding(event.target.checked);
                  }}
                />
                <span>
                  {t('conversation.fold.setting')}
                  <small>{t('conversation.fold.settingDetail')}</small>
                </span>
              </label>
            ) : null}
            <p>{t('conversation.runtime')}</p>
            <p>{t('conversation.help')}</p>
          </div>
        </details>
      </div>
      {belowBar}
      {timeline.length === 0 ? null : (
        <ConversationSearch
          id={searchId}
          hidden={!searchVisible}
          messages={timeline}
          filter={filter}
          onChange={setFilter}
          count={visibleTimeline.length}
        />
      )}
      {positionMissing || searching ? (
        <div className="conversation-history-tools">
          {positionMissing ? <p role="status">{t('history.positionMissing')}</p> : null}
          <button
            type="button"
            onClick={() => {
              setFilter(emptyConversationFilter);
              setJump(null);
              position.latest();
              requestAnimationFrame(() => {
                if (timelineElement.current)
                  timelineElement.current.scrollTop = timelineElement.current.scrollHeight;
              });
            }}
          >
            {t('history.latest')}
          </button>
        </div>
      ) : null}
      <div className="conversation-panel__history">
        <div
          className="conversation-panel__timeline"
          ref={timelineElement}
          onScroll={(event) => {
            const element = event.currentTarget;
            following.current = atLatest(element);
            if (bottomDistance.current !== null)
              bottomDistance.current = element.scrollHeight - element.scrollTop;
            if (following.current) setUnseen(false);
            markLatestDisplayed();
            position.record();
          }}
          role="log"
          // 面板矮时时间线会滚动；可滚动区域必须能用键盘聚焦，否则只用键盘的人翻不到旧消息。
          tabIndex={0}
          aria-label={t('conversation.title')}
          aria-live="polite"
          aria-relevant="additions text"
          aria-busy={state === 'loading'}
        >
          {state === 'ready' && (timeline.length > 0 || undecryptable !== undefined) ? (
            <HistoryBoundary history={history} state={historyState} onLoad={loadOlder} />
          ) : null}
          {state === 'ready' && undecryptable !== undefined ? (
            <UndecryptableNotice summary={undecryptable} names={names} recovery={recovery} />
          ) : null}
          {state === 'loading' ? (
            <p className="conversation-panel__boundary">{t('conversation.loading')}</p>
          ) : state === 'failed' ? (
            <p className="conversation-panel__boundary" role="alert">
              {t('conversation.unavailable')}
            </p>
          ) : timeline.length === 0 && undecryptable === undefined ? (
            <motion.div
              className="conversation-panel__empty"
              initial={reduceMotion === true ? false : { opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ type: 'spring', stiffness: 300, damping: 30 }}
            >
              <h3>{t('conversation.emptyTitle')}</h3>
              <p>{t('conversation.empty')}</p>
            </motion.div>
          ) : null}
          {filtered && timeline.length > 0 && visibleTimeline.length === 0 ? (
            <p className="conversation-panel__boundary">
              {t(
                searching
                  ? 'history.empty'
                  : scope === 'humans'
                    ? 'conversation.scope.emptyHumans'
                    : 'conversation.scope.emptyAddressed',
              )}
            </p>
          ) : null}
          {items.map((item, index) => {
            const at = itemStart(item);
            const previous = items[index - 1];
            const separator =
              previous === undefined ||
              new Date(itemEnd(previous)).toDateString() !== new Date(at).toDateString() ? (
                <div className="conversation-day">
                  <span>{date.format(at)}</span>
                </div>
              ) : null;
            if (item.kind === 'notice') {
              return (
                <div
                  className="conversation-panel__notice-entry"
                  key={`notice:${item.change.matrixEventId}`}
                >
                  {separator}
                  <RoomNoticeChangeLine
                    change={item.change}
                    names={names}
                    selfMatrixUserId={selfId}
                    time={time}
                  />
                </div>
              );
            }
            if (item.kind === 'message')
              return renderMessage(
                item.message,
                item.message.messageId === lineBefore ? (
                  <>
                    {separator}
                    {newLine}
                  </>
                ) : (
                  separator
                ),
              );
            const open = openFolds.get(item.key) ?? false;
            // 新消息从这一串里的哪一句起：折着的时候，或者就是第一句，线画在这一行上面；
            // 展开了、又是中间的某一句，线画在那一句上面。
            const newIndex =
              lineBefore === null
                ? -1
                : item.messages.findIndex((message) => message.messageId === lineBefore);
            const lineAbove = newIndex === 0 || (newIndex > 0 && !open);
            return (
              <Fragment key={`fold:${item.key}`}>
                <ConversationFoldRow
                  fold={item}
                  open={open}
                  separator={
                    lineAbove ? (
                      <>
                        {separator}
                        {newLine}
                      </>
                    ) : (
                      separator
                    )
                  }
                  onToggle={() => {
                    setOpenFolds((current) => new Map(current).set(item.key, !open));
                    // 这一串在最下面那几句的上面：展开、收起以后人多半不在底下了，重新看一眼，
                    // 免得来了新消息把他从正在读的地方拉走。
                    requestAnimationFrame(() => {
                      const element = timelineElement.current;
                      if (element === null) return;
                      following.current = atLatest(element);
                      position.record();
                    });
                  }}
                />
                {open
                  ? item.messages.map((message, at) =>
                      renderMessage(message, at > 0 && at === newIndex ? newLine : null),
                    )
                  : null}
              </Fragment>
            );
          })}
        </div>
        {unseen ? (
          <button
            className="conversation-panel__latest"
            type="button"
            onClick={() => {
              const element = timelineElement.current;
              if (element !== null) element.scrollTop = element.scrollHeight;
              following.current = true;
              setJump(null);
              position.latest();
              setUnseen(false);
              markLatestDisplayed();
            }}
          >
            <ArrowDown aria-hidden="true" />
            {waiting > 0 ? t('conversation.newCount', { count: waiting }) : t('history.latest')}
          </button>
        ) : null}
      </div>
      <ConversationComposer
        composer={composer}
        participants={participants}
        names={names}
        roomId={roomId}
        canSend={canSend}
        writesAllowed={writesAllowed}
        input={input}
        typing={typing}
        allowEveryone={privateRoom && variant === 'room'}
      />
    </section>
  );
}

/** 滚在最底下（差不到一行）：跟着最新的消息走。 */
function atLatest(element: HTMLElement): boolean {
  return element.scrollHeight - element.scrollTop - element.clientHeight < 48;
}

function itemStart(item: ConversationItem): number {
  if (item.kind === 'message') return item.message.serverTimestamp;
  if (item.kind === 'notice') return item.change.serverTimestamp;
  return item.messages[0]?.serverTimestamp ?? 0;
}

function itemEnd(item: ConversationItem): number {
  return item.kind === 'fold' ? (item.messages.at(-1)?.serverTimestamp ?? 0) : itemStart(item);
}

/** 最多点名这么多个发送者，再多就只说人数。 */
const maxNamedUndecryptableSenders = 3;

function UndecryptableNotice({
  summary,
  names,
  recovery,
}: {
  readonly summary: UndecryptableSummary;
  readonly names: ReadonlyMap<string, string>;
  readonly recovery: UndecryptableRecovery;
}) {
  const { t } = useTranslation();
  const named = summary.senders
    .slice(0, maxNamedUndecryptableSenders)
    .map((sender) => names.get(sender) ?? sender);
  const others = summary.senders.length - named.length;
  return (
    <Banner
      className="conversation-undecryptable"
      icon={<LockKeyhole />}
      role="status"
      title={t('conversation.undecryptable.title', { count: summary.count })}
      tone="warning"
    >
      {named.length === 0 ? null : (
        <p>
          {others > 0
            ? t('conversation.undecryptable.sendersMore', {
                count: others,
                names: named.join(t('conversation.undecryptable.separator')),
              })
            : t('conversation.undecryptable.senders', {
                names: named.join(t('conversation.undecryptable.separator')),
              })}
        </p>
      )}
      {summary.reasons.map((reason) => (
        <p key={reason}>{t(`conversation.undecryptable.reason.${reason}`)}</p>
      ))}
      {recovery === 'requested' ? <p>{t('conversation.undecryptable.recovering')}</p> : null}
      {recovery === 'awaiting_signing' ? (
        <p>{t('conversation.undecryptable.awaitingSigning')}</p>
      ) : null}
    </Banner>
  );
}

function HistoryBoundary({
  history,
  state,
  onLoad,
}: {
  readonly history: MessageRoomProjection['history'];
  readonly state: 'ready' | 'loading' | 'failed';
  readonly onLoad: (() => void) | undefined;
}) {
  const { t } = useTranslation();
  if (history === undefined && state !== 'failed') return null;
  return (
    <div className="conversation-history-boundary">
      {history?.canLoadMore && onLoad ? (
        <button type="button" disabled={state === 'loading'} onClick={onLoad}>
          {t(state === 'loading' ? 'history.loading' : 'history.older')}
        </button>
      ) : null}
      {history?.limited ? (
        <span>{t('history.limit')}</span>
      ) : history && !history.canLoadMore ? (
        <span>{t('history.complete')}</span>
      ) : null}
      {state === 'failed' ? <p role="alert">{t('history.failed')}</p> : null}
    </div>
  );
}
