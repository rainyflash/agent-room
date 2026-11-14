// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { agentRunFoldingEnabled } from '@/features/conversation/application/agent-run-folding';
import type { ConversationParticipant } from '@/features/conversation/domain/conversation';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import { BrowserWorkspaceCache } from '@/features/personal-workspace/adapters/browser-workspace-cache';
import { PersonalWorkspaceStore } from '@/features/personal-workspace/application/personal-workspace-store';
import {
  changeWorkspace,
  emptyWorkspace,
  type WorkspaceDocument,
} from '@/features/personal-workspace/domain/workspace-document';
import { PersonalWorkspaceProvider } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { ok } from '@/shared/result';

import { ConversationPanel } from './conversation-panel';

const ROOM = '!chat:agent-room.test';
const YOU = '@rainy:agent-room.test';
const GRACE = '@grace:agent-room.test';
const ATLAS = '@atlas:agent-room.test';
const COMET = '@comet:agent-room.test';
const NOVA = '@nova:agent-room.test';
const agentIds = [ATLAS, COMET, NOVA];
const NOON = new Date(2026, 10, 14, 12).getTime();
const names: Readonly<Record<string, string>> = {
  [YOU]: 'Rainy',
  [GRACE]: 'Grace',
  [ATLAS]: 'Atlas',
  [COMET]: 'Comet',
  [NOVA]: 'Nova',
};

// jsdom 不排版，没有这个方法；去看某一条消息时面板会调它。
const scrolled = vi.fn();
beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
  Element.prototype.scrollIntoView = scrolled;
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.removeItem('agent-room.conversation.agent-run-folding');
});

type Line = {
  readonly by: string;
  readonly text: string;
  readonly mentions?: readonly string[];
  readonly replyTo?: number;
};

function messageId(index: number): string {
  return `01990d9e-8400-7000-8000-${String(3000 + index).padStart(12, '0')}`;
}

function conversation(lines: readonly Line[]): readonly RoomMessageSignal[] {
  return lines.map((line, index): RoomMessageSignal => {
    const agent = agentIds.includes(line.by);
    return {
      ...conversationFixture(messageId(index), {
        roomId: ROOM,
        serverTimestamp: NOON + index * 60_000,
      }),
      actor: agent
        ? {
            kind: 'agent',
            agentId: `01990d9e-8400-7000-8000-00000000000${String(agentIds.indexOf(line.by) + 1)}`,
            instanceId: `instance-${line.by}`,
            displayName: names[line.by] ?? line.by,
            matrixUserId: line.by,
            provenance: 'autonomous_agent',
          }
        : {
            kind: 'human',
            principalId: `principal-${line.by}`,
            displayName: names[line.by] ?? line.by,
            matrixUserId: line.by,
            provenance: 'human',
          },
      preview: {
        title: 'Conversation',
        summary: line.text,
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: line.text, mentions: line.mentions ?? [] },
      },
      ...(line.replyTo === undefined
        ? {}
        : { relation: { kind: 'reply' as const, targetMessageId: messageId(line.replyTo) } }),
    };
  });
}

/** 你问了一句，Atlas 回你；Atlas 和 Comet 之间又说了 5 句；Grace 点了你；最后 Comet 说了两句。 */
const lines: readonly Line[] = [
  { by: YOU, text: 'Check the vendored crate.', mentions: [ATLAS] },
  { by: ATLAS, text: 'One gap in the image builds.', replyTo: 0 },
  { by: COMET, text: 'I can add the copy step.' },
  { by: ATLAS, text: 'Start with the control plane.', replyTo: 2 },
  { by: COMET, text: 'Control plane builds now.', mentions: [ATLAS] },
  { by: ATLAS, text: 'The gateway misses a file.' },
  { by: COMET, text: 'Added the file.' },
  { by: GRACE, text: 'Merged, have a look.', mentions: [YOU] },
  { by: COMET, text: 'Drafting the notes.' },
  { by: COMET, text: 'Notes are ready.' },
];
const foldLabel = '5 more messages between Comet and Atlas';
const agents: readonly ConversationParticipant[] = [
  { matrixUserId: ATLAS, displayName: 'Atlas' },
  { matrixUserId: COMET, displayName: 'Comet' },
];

const publisher: MessagePublisher = {
  publish: () => Promise.reject(new Error('这些用例不发消息。')),
  reconcile: () => Promise.reject(new Error('这些用例不发消息。')),
  resolveIdentity: () =>
    Promise.resolve(
      ok({
        kind: 'human',
        displayName: 'Rainy',
        matrixUserId: YOU,
        principalId: '01990d9e-8400-7000-8000-000000000009',
        source: 'matrix_human_session',
      }),
    ),
};

/** 这个账户在这间房读到了第几句。 */
function readUpTo(messages: readonly RoomMessageSignal[], index: number): PersonalWorkspaceStore {
  const target = messages[index];
  if (target === undefined) throw new Error('没有这一句。');
  const changed = changeWorkspace(
    emptyWorkspace,
    {
      kind: 'read',
      id: ROOM,
      value: { eventId: target.matrixEventId, timestamp: target.serverTimestamp },
    },
    'another-device',
  );
  if (!changed.ok) throw new Error('读到的位置写不进去。');
  let remote: WorkspaceDocument = changed.value;
  const values = new Map<string, string>();
  return new PersonalWorkspaceStore(
    {
      scope: () => ({ accountId: YOU, writerId: 'this-device' }),
      read: () => Promise.resolve(ok(remote)),
      write: (_scope, document) => {
        remote = document;
        return Promise.resolve(ok(undefined));
      },
      subscribe: () => () => undefined,
    },
    new BrowserWorkspaceCache({
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => {
        values.set(key, value);
      },
    }),
  );
}

async function renderPanel(
  messages: readonly RoomMessageSignal[],
  options: {
    readonly personal?: PersonalWorkspaceStore;
    readonly variant?: 'room' | 'direct';
    readonly focusMessageId?: string;
    readonly barEnd?: ReactNode;
    readonly onLatestDisplayed?: (eventId: string) => void;
  } = {},
) {
  const panel = (
    focusMessageId: string | undefined,
    shownMessages: readonly RoomMessageSignal[] = messages,
  ): ReactNode => {
    const element = (
      <ConversationPanel
        messages={shownMessages}
        participants={agents}
        publisher={publisher}
        roomId={ROOM}
        state="ready"
        {...(options.variant === undefined ? {} : { variant: options.variant })}
        {...(focusMessageId === undefined ? {} : { focusMessageId })}
        {...(options.barEnd === undefined ? {} : { barEnd: options.barEnd })}
        {...(options.onLatestDisplayed === undefined
          ? {}
          : { onLatestDisplayed: options.onLatestDisplayed })}
      />
    );
    return (
      <I18nextProvider i18n={i18n}>
        {options.personal === undefined ? (
          element
        ) : (
          <PersonalWorkspaceProvider store={options.personal}>{element}</PersonalWorkspaceProvider>
        )}
      </I18nextProvider>
    );
  };
  const view = render(panel(options.focusMessageId));
  // 知道了你是谁，才分得出哪句是你说的、哪句在找你。
  await waitFor(() => {
    expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
  });
  return {
    /** 去看某一条消息；不给就是不再指着哪一条。 */
    focus: (focusMessageId?: string) => {
      view.rerender(panel(focusMessageId));
    },
    /** 对话变成了这样：来了新消息，或者往前补出了旧的。 */
    change: (next: readonly RoomMessageSignal[]) => {
      view.rerender(panel(options.focusMessageId, next));
    },
  };
}

const log = () => screen.getByRole('log', { name: 'Conversation' });
const shown = (): string[] =>
  [...log().querySelectorAll('.conversation-message__text p')].map(
    (paragraph) => paragraph.textContent,
  );
const scope = (name: string) =>
  within(screen.getByRole('group', { name: 'Show' })).getByRole('button', { name });

describe('只看：全部、人说的、提到我的', () => {
  it('人说的只留人说的；提到我的只留点了你、回复你的；再点全部都回来', async () => {
    const user = userEvent.setup();
    window.localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
    await renderPanel(conversation(lines));
    expect(scope('All')).toHaveAttribute('aria-pressed', 'true');
    expect(shown()).toHaveLength(10);

    await user.click(scope('From people'));
    expect(scope('From people')).toHaveAttribute('aria-pressed', 'true');
    expect(scope('All')).toHaveAttribute('aria-pressed', 'false');
    expect(shown()).toEqual(['Check the vendored crate.', 'Merged, have a look.']);

    await user.click(scope('Mentions me'));
    expect(shown()).toEqual(['One gap in the image builds.', 'Merged, have a look.']);

    await user.click(scope('All'));
    expect(shown()).toHaveLength(10);
  });

  it('在提到我的里说了话：回到全部，刚说的那句看得到；在人说的里不动', async () => {
    const user = userEvent.setup();
    window.localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
    const said = conversation([...lines, { by: YOU, text: 'On it.' }]);
    const before = said.slice(0, -1);
    const panel = await renderPanel(before);
    await user.click(scope('Mentions me'));
    expect(shown()).toHaveLength(2);
    // 别人说的话、往前补出来的你以前说的话，都不动。
    panel.change(before.slice(1));
    panel.change(before);
    expect(scope('Mentions me')).toHaveAttribute('aria-pressed', 'true');
    panel.change(said);
    expect(scope('All')).toHaveAttribute('aria-pressed', 'true');
    expect(shown()).toHaveLength(11);
    expect(shown().at(-1)).toBe('On it.');
    cleanup();

    const people = await renderPanel(before);
    await user.click(scope('From people'));
    people.change(said);
    expect(scope('From people')).toHaveAttribute('aria-pressed', 'true');
    expect(shown().at(-1)).toBe('On it.');
  });

  it('筛出来一条也没有时各说一句，不说“还没有消息”', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation([{ by: ATLAS, text: 'Standing by.' }]));
    await user.click(scope('From people'));
    expect(screen.getByText('No person has said anything here yet.')).toBeVisible();
    await user.click(scope('Mentions me'));
    expect(screen.getByText('Nothing here mentions you or replies to you yet.')).toBeVisible();
    expect(screen.queryByText('No messages yet.')).not.toBeInTheDocument();
  });

  it('提到我的带着没读的条数：只数读到的位置以后的，按钮的名字不变', async () => {
    const messages = conversation(lines);
    // 读到了你自己那一句：后面 Atlas 回你、Grace 点你，两条没读。
    await renderPanel(messages, { personal: readUpTo(messages, 0) });
    await waitFor(() => {
      expect(scope('Mentions me')).toHaveAccessibleDescription('2 unread');
    });
    expect(scope('Mentions me')).toHaveTextContent('Mentions me2');
    cleanup();

    // 读到了最后：没有没读的，不画数字。
    await renderPanel(messages, { personal: readUpTo(messages, 9) });
    expect(scope('Mentions me')).toHaveTextContent(/^Mentions me$/u);
    expect(scope('Mentions me')).not.toHaveAccessibleDescription();
  });

  it('私聊里只有两方：不分“只看”，也不折', async () => {
    await renderPanel(conversation(lines), { variant: 'direct' });
    expect(screen.queryByRole('group', { name: 'Show' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: foldLabel })).not.toBeInTheDocument();
    expect(shown()).toHaveLength(10);
    // 详情里也没有折不折的开关。
    expect(
      screen.queryByRole('checkbox', { name: /Fold long runs between agents/u }),
    ).not.toBeInTheDocument();
  });

  it('顶上那一行右边能放别的按钮', async () => {
    await renderPanel(conversation(lines), { barEnd: <button type="button">Resources</button> });
    expect(screen.getByRole('button', { name: 'Resources' })).toBeVisible();
  });
});

describe('Agent 之间的长串折起来', () => {
  it('中间那一串折成一行，写着谁和谁、几句；最下面那一串不折', async () => {
    await renderPanel(conversation(lines));
    const fold = screen.getByRole('button', { name: foldLabel });
    expect(fold).toHaveAttribute('aria-expanded', 'false');
    expect(shown()).toEqual([
      'Check the vendored crate.',
      'One gap in the image builds.',
      'Merged, have a look.',
      'Drafting the notes.',
      'Notes are ready.',
    ]);
  });

  it('点一下展开，看得到每一句；再点收起', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(lines));
    await user.click(screen.getByRole('button', { name: foldLabel }));
    expect(screen.getByRole('button', { name: foldLabel })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    expect(shown()).toHaveLength(10);
    expect(shown().slice(2, 7)).toEqual([
      'I can add the copy step.',
      'Start with the control plane.',
      'Control plane builds now.',
      'The gateway misses a file.',
      'Added the file.',
    ]);
    await user.click(screen.getByRole('button', { name: foldLabel }));
    expect(shown()).toHaveLength(5);
  });

  it('详情里关掉就都摆出来，记在这台设备上；再打开又折回去', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(lines));
    await user.click(
      screen.getByText('', { selector: 'summary[aria-label="Conversation details"]' }),
    );
    const toggle = screen.getByRole('checkbox', { name: /Fold long runs between agents/u });
    expect(toggle).toBeChecked();
    await user.click(toggle);
    expect(screen.queryByRole('button', { name: foldLabel })).not.toBeInTheDocument();
    expect(shown()).toHaveLength(10);
    expect(agentRunFoldingEnabled()).toBe(false);

    await user.click(toggle);
    expect(screen.getByRole('button', { name: foldLabel })).toBeVisible();
    expect(agentRunFoldingEnabled()).toBe(true);
  });

  it('这台设备关掉过：打开就不折', async () => {
    window.localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
    await renderPanel(conversation(lines));
    expect(screen.queryByRole('button', { name: foldLabel })).not.toBeInTheDocument();
    expect(shown()).toHaveLength(10);
  });

  it('要去看的那条折在里面：那一串自己打开；正筛着时先回到全部', async () => {
    const user = userEvent.setup();
    const view = await renderPanel(conversation(lines));
    await user.click(scope('From people'));
    expect(shown()).toHaveLength(2);

    view.focus(messageId(4));
    await waitFor(() => {
      expect(scope('All')).toHaveAttribute('aria-pressed', 'true');
    });
    expect(screen.getByRole('button', { name: foldLabel })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    const focused = log().querySelector('[data-focused="true"]');
    expect(focused).toHaveTextContent('Control plane builds now.');

    // 不再指着那一句了，正读着的这一串不自己缩回去。
    const fold = screen.getByRole('button', { name: foldLabel });
    view.focus();
    expect(fold).toHaveAttribute('aria-expanded', 'true');
    // 人自己收起来以后，再指着里面另一句，又替他打开；指着的时候收起来就不再打开。
    await user.click(fold);
    expect(fold).toHaveAttribute('aria-expanded', 'false');
    view.focus(messageId(5));
    expect(fold).toHaveAttribute('aria-expanded', 'true');
    expect(log().querySelector('[data-focused="true"]')).toHaveTextContent(
      'The gateway misses a file.',
    );
    await user.click(fold);
    expect(fold).toHaveAttribute('aria-expanded', 'false');
    view.change(conversation([...lines, { by: GRACE, text: 'One more thing.' }]));
    expect(fold).toHaveAttribute('aria-expanded', 'false');
  });

  it('查找、筛选的时候不折：找到的每一句都摆出来', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(lines));
    await user.click(screen.getByRole('button', { name: 'Search this conversation' }));
    await user.type(screen.getByRole('searchbox', { name: 'Keyword' }), 'control plane');
    await waitFor(() => {
      expect(shown()).toEqual(['Start with the control plane.', 'Control plane builds now.']);
    });
    expect(screen.queryByRole('button', { name: foldLabel })).not.toBeInTheDocument();
  });

  it('只有一个 Agent 自己连着说、三个以上一起说，各有各的写法', async () => {
    await renderPanel(
      conversation([
        ...Array.from({ length: 4 }, (_, index) => ({ by: ATLAS, text: `Step ${String(index)}` })),
        { by: GRACE, text: 'Thanks.' },
      ]),
    );
    expect(screen.getByRole('button', { name: '4 more messages from Atlas' })).toBeVisible();
    cleanup();

    await renderPanel(
      conversation([
        { by: ATLAS, text: 'One.' },
        { by: COMET, text: 'Two.' },
        { by: NOVA, text: 'Three.' },
        { by: ATLAS, text: 'Four.' },
        { by: NOVA, text: 'Five.' },
        { by: GRACE, text: 'Thanks.' },
      ]),
    );
    expect(screen.getByRole('button', { name: '5 more messages between 3 agents' })).toBeVisible();
  });
});

/** jsdom 不排版：给对话那一块一个高度，内容比它高，才分得出人在不在最底下。 */
function layout(): () => void {
  const sized = (element: Element, value: number) =>
    element.classList.contains('conversation-panel__timeline') ? value : 0;
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get(this: HTMLElement) {
      return sized(this, 500);
    },
  });
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
    configurable: true,
    get(this: HTMLElement) {
      return sized(this, 2000);
    },
  });
  return () => {
    Reflect.deleteProperty(HTMLElement.prototype, 'clientHeight');
    Reflect.deleteProperty(HTMLElement.prototype, 'scrollHeight');
  };
}
const scrollUp = () => {
  log().scrollTop = 0;
  fireEvent.scroll(log());
};
/** 窗口到前台、离开前台。 */
function windowFocus(): (focused: boolean) => void {
  const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  return (focused) => {
    hasFocus.mockReturnValue(focused);
    fireEvent(window, new Event(focused ? 'focus' : 'blur'));
  };
}
const newLine = () => screen.queryByRole('separator', { name: 'New' });
/** “新消息”那条线下面紧跟着的是什么。 */
const underLine = () => newLine()?.nextElementSibling ?? null;
const latestButton = () => document.querySelector('.conversation-panel__latest');
const focused = () => log().querySelector('[data-focused="true"]');

describe('“新消息”那条线', () => {
  it('打开时画在读到的位置后面第一句别人说的上面；都读了、没有读到的位置就不画', async () => {
    const messages = conversation(lines);
    // 读到了 Grace 那一句：后面 Comet 的两句是新的。
    await renderPanel(messages, { personal: readUpTo(messages, 7) });
    await waitFor(() => {
      expect(newLine()).toBeInTheDocument();
    });
    expect(underLine()).toHaveTextContent('Drafting the notes.');
    expect(screen.getAllByRole('separator', { name: 'New' })).toHaveLength(1);
    cleanup();

    await renderPanel(messages, { personal: readUpTo(messages, 9) });
    expect(scope('Mentions me')).toBeVisible();
    expect(newLine()).not.toBeInTheDocument();
    cleanup();

    await renderPanel(messages);
    expect(newLine()).not.toBeInTheDocument();
  });

  it('新的从折起来的一串中间起：线画在那一行上面；展开以后画在那一句上面', async () => {
    const user = userEvent.setup();
    const messages = conversation(lines);
    await renderPanel(messages, { personal: readUpTo(messages, 3) });
    await waitFor(() => {
      expect(newLine()).toBeInTheDocument();
    });
    const fold = screen.getByRole('button', { name: foldLabel });
    expect(underLine()).toBe(fold);

    await user.click(fold);
    expect(underLine()).toHaveTextContent('Control plane builds now.');
    expect(screen.getAllByRole('separator', { name: 'New' })).toHaveLength(1);
    cleanup();

    // 就是这一串的第一句：展开了线也留在那一行上面。
    await renderPanel(messages, { personal: readUpTo(messages, 1) });
    await waitFor(() => {
      expect(newLine()).toBeInTheDocument();
    });
    await user.click(screen.getByRole('button', { name: foldLabel }));
    expect(underLine()).toBe(screen.getByRole('button', { name: foldLabel }));
  });

  it('看到底的那一刻线定下来：读到的位置往前走了，线留在原处', async () => {
    window.localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
    const restore = layout();
    try {
      windowFocus();
      const messages = conversation(lines);
      const personal = readUpTo(messages, 7);
      await renderPanel(messages, { personal });
      // 这台设备上没记过，读到的位置是从别处同步下来的：等它到了才记“读到最新了”。
      await waitFor(() => {
        expect(personal.getSnapshot().index.read.get(ROOM)?.eventId).toBe(
          messages.at(-1)?.matrixEventId,
        );
      });
      expect(underLine()).toHaveTextContent('Drafting the notes.');
      expect(latestButton()).not.toBeInTheDocument();
    } finally {
      restore();
    }
  });

  it('正在筛、查找时不画；你说了话就撤掉', async () => {
    const user = userEvent.setup();
    const messages = conversation(lines);
    const panel = await renderPanel(messages, { personal: readUpTo(messages, 7) });
    await waitFor(() => {
      expect(newLine()).toBeInTheDocument();
    });
    await user.click(scope('From people'));
    expect(newLine()).not.toBeInTheDocument();
    await user.click(scope('All'));
    expect(underLine()).toHaveTextContent('Drafting the notes.');

    panel.change(conversation([...lines, { by: YOU, text: 'On it.' }]));
    expect(newLine()).not.toBeInTheDocument();
  });

  it('没在最底下时来了新的：按钮写几条，线画在这一批的第一句上面；到底以后线留着，再来一批才挪', async () => {
    const user = userEvent.setup();
    window.localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
    const restore = layout();
    try {
      windowFocus();
      const panel = await renderPanel(conversation(lines));
      expect(newLine()).not.toBeInTheDocument();
      expect(latestButton()).not.toBeInTheDocument();

      scrollUp();
      const first = [...lines, { by: ATLAS, text: 'New one.' }, { by: GRACE, text: 'New two.' }];
      panel.change(conversation(first));
      expect(latestButton()).toHaveTextContent('2 new messages');
      expect(underLine()).toHaveTextContent('New one.');

      const second = [...first, { by: COMET, text: 'New three.' }];
      panel.change(conversation(second));
      expect(latestButton()).toHaveTextContent('3 new messages');
      expect(underLine()).toHaveTextContent('New one.');

      await user.click(screen.getByRole('button', { name: '3 new messages' }));
      expect(latestButton()).not.toBeInTheDocument();
      expect(underLine()).toHaveTextContent('New one.');

      scrollUp();
      panel.change(conversation([...second, { by: ATLAS, text: 'New four.' }]));
      expect(latestButton()).toHaveTextContent('1 new message');
      expect(underLine()).toHaveTextContent('New four.');
      expect(screen.getAllByRole('separator', { name: 'New' })).toHaveLength(1);
    } finally {
      restore();
    }
  });

  it('筛着的时候按钮不写条数：数的是整个对话里的', async () => {
    const user = userEvent.setup();
    const restore = layout();
    try {
      windowFocus();
      const panel = await renderPanel(conversation(lines));
      await user.click(scope('From people'));
      scrollUp();
      panel.change(conversation([...lines, { by: ATLAS, text: 'New one.' }]));
      expect(latestButton()).toHaveTextContent('Return to latest');
      expect(newLine()).not.toBeInTheDocument();
    } finally {
      restore();
    }
  });

  it('窗口不在前台时来的话不算看到：线画在它上面，回到前台才记成看过', async () => {
    window.localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
    const restore = layout();
    try {
      const focusWindow = windowFocus();
      const displayed = vi.fn();
      const messages = conversation(lines);
      const panel = await renderPanel(messages, { onLatestDisplayed: displayed });
      expect(displayed).toHaveBeenLastCalledWith(messages.at(-1)?.matrixEventId);

      focusWindow(false);
      const away = conversation([...lines, { by: ATLAS, text: 'While you were away.' }]);
      panel.change(away);
      expect(underLine()).toHaveTextContent('While you were away.');
      expect(displayed).toHaveBeenCalledTimes(1);
      // 人还在最底下，只是没看着：不用按钮。
      expect(latestButton()).not.toBeInTheDocument();

      focusWindow(true);
      expect(displayed).toHaveBeenLastCalledWith(away.at(-1)?.matrixEventId);
      // 回来了线还在，看得出是从哪儿开始没看到的。
      expect(underLine()).toHaveTextContent('While you were away.');

      // 看着的时候来的不挪线。
      panel.change(
        conversation([
          ...lines,
          { by: ATLAS, text: 'While you were away.' },
          { by: COMET, text: 'Seen live.' },
        ]),
      );
      expect(underLine()).toHaveTextContent('While you were away.');
    } finally {
      restore();
    }
  });
});

describe('点回复的是哪句，跳到那一句', () => {
  const withReply: readonly Line[] = [
    ...lines,
    { by: GRACE, text: 'About the gateway file.', replyTo: 5 },
  ];
  const quote = (name: string) => screen.getByRole('button', { name });

  it('跳过去标出来；同一句再点一次再跳一次', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(withReply));
    scrolled.mockClear();
    await user.click(quote('Rainy: Check the vendored crate.'));
    expect(focused()).toHaveTextContent('Check the vendored crate.');
    expect(focused()).toHaveFocus();
    expect(scrolled).toHaveBeenCalledTimes(1);

    await user.click(quote('Rainy: Check the vendored crate.'));
    expect(scrolled).toHaveBeenCalledTimes(2);
    expect(focused()).toHaveTextContent('Check the vendored crate.');
  });

  it('那一句折着：那一串自己打开', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(withReply));
    expect(screen.getByRole('button', { name: foldLabel })).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    await user.click(quote('Atlas: The gateway misses a file.'));
    expect(screen.getByRole('button', { name: foldLabel })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    expect(focused()).toHaveTextContent('The gateway misses a file.');
    // 只框住那一句，日期和别的句子不在框里。
    expect(focused()?.querySelectorAll('.conversation-message__text p')).toHaveLength(1);
  });

  it('正筛着、查找着：先放开，回到完整的对话', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(withReply));
    await user.click(scope('Mentions me'));
    expect(shown()).toEqual(['One gap in the image builds.', 'Merged, have a look.']);
    await user.click(quote('Rainy: Check the vendored crate.'));
    expect(scope('All')).toHaveAttribute('aria-pressed', 'true');
    expect(focused()).toHaveTextContent('Check the vendored crate.');

    await user.click(screen.getByRole('button', { name: 'Search this conversation' }));
    await user.type(screen.getByRole('searchbox', { name: 'Keyword' }), 'gateway file');
    await waitFor(() => {
      expect(shown()).toEqual(['About the gateway file.']);
    });
    await user.click(quote('Atlas: The gateway misses a file.'));
    await waitFor(() => {
      expect(focused()).toHaveTextContent('The gateway misses a file.');
    });
    expect(screen.queryByRole('searchbox', { name: 'Keyword' })).not.toBeInTheDocument();
  });

  it('换了“只看”、你说了话、外面指了另一句：不再标着跳过去的那一句', async () => {
    const user = userEvent.setup();
    const panel = await renderPanel(conversation(withReply));
    await user.click(quote('Rainy: Check the vendored crate.'));
    expect(focused()).toHaveTextContent('Check the vendored crate.');
    await user.click(scope('From people'));
    expect(focused()).not.toBeInTheDocument();

    await user.click(scope('All'));
    await user.click(quote('Rainy: Check the vendored crate.'));
    expect(focused()).toHaveTextContent('Check the vendored crate.');
    panel.change(conversation([...withReply, { by: YOU, text: 'On it.' }]));
    expect(focused()).not.toBeInTheDocument();
    cleanup();

    const linked = await renderPanel(conversation(withReply));
    await user.click(quote('Rainy: Check the vendored crate.'));
    linked.focus(messageId(7));
    expect(focused()).toHaveTextContent('Merged, have a look.');
    // 外面不再指着了，之前跳过去的那一句也不回来。
    linked.focus();
    expect(focused()).not.toBeInTheDocument();
  });

  it('私聊里也能跳', async () => {
    const user = userEvent.setup();
    await renderPanel(conversation(withReply), { variant: 'direct' });
    await user.click(quote('Rainy: Check the vendored crate.'));
    expect(focused()).toHaveTextContent('Check the vendored crate.');
  });
});
