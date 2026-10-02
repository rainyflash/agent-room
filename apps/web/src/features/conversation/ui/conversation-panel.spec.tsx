// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { ConversationPanel } from './conversation-panel';
import type {
  MessagePublicationRequest,
  MessagePublicationResult,
  MessagePublisher,
} from '@/features/messages/domain/publication';
import type {
  RoomMessageSignal,
  UndecryptableRecovery,
  UndecryptableSummary,
} from '@/features/messages/domain/message';
import { NetworkAgentLabelStore } from '@/features/lobby/application/network-agent-label-store';
import { NetworkAgentLabelsProvider } from '@/features/lobby/ui/network-agent-labels';
import { initializeI18n, i18n } from '@/shared/i18n/i18n';
import { err, ok, type Result } from '@/shared/result';

const roomId = '!chat:agent-room.test';
const submissionId = '01990d9e-8400-7000-8000-000000000003';
const agentId = '@assistant:agent-room.test';
beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(cleanup);

function harness(
  unknown = false,
  messages: readonly RoomMessageSignal[] = [],
  labels: NetworkAgentLabelStore | null = null,
  undecryptable?: UndecryptableSummary,
  recovery: UndecryptableRecovery = 'idle',
) {
  const publish = vi.fn((request: MessagePublicationRequest): Promise<MessagePublicationResult> =>
    Promise.resolve(
      ok(
        unknown
          ? {
              kind: 'pending_reconciliation' as const,
              submissionId: request.submissionId,
              transactionId: `agent-room-message-${request.submissionId}`,
            }
          : {
              kind: 'published' as const,
              matrixEventId: '$sent',
              reused: false,
              submissionId: request.submissionId,
            },
      ),
    ),
  );
  const reconcile = vi.fn((id: string) =>
    Promise.resolve(
      ok({ kind: 'published' as const, matrixEventId: '$sent', reused: true, submissionId: id }),
    ),
  );
  const publisher: MessagePublisher = {
    publish,
    reconcile,
    resolveIdentity: () =>
      Promise.resolve(
        ok({
          kind: 'human',
          displayName: 'Rainy',
          matrixUserId: '@rainy:agent-room.test',
          principalId: submissionId,
          source: 'matrix_human_session',
        }),
      ),
  };
  const panel = (
    <ConversationPanel
      messages={messages}
      publisher={publisher}
      roomId={roomId}
      state="ready"
      participants={[{ matrixUserId: agentId, displayName: 'Ada' }]}
      submissionIds={{ next: () => submissionId }}
      {...(undecryptable === undefined ? {} : { recovery, undecryptable })}
    />
  );
  render(
    <I18nextProvider i18n={i18n}>
      {labels === null ? (
        panel
      ) : (
        <NetworkAgentLabelsProvider store={labels}>{panel}</NetworkAgentLabelsProvider>
      )}
    </I18nextProvider>,
  );
  return { publish, reconcile };
}

describe('解不开的加密消息', () => {
  it('房间里只有解不开的消息时说清楚有多少条、来自谁、为什么，不说“还没有消息”', () => {
    harness(false, [], null, {
      count: 329,
      reasons: ['missing_key', 'withheld'],
      senders: [
        agentId,
        '@builder:agent-room.test',
        '@tester:agent-room.test',
        '@writer:agent-room.test',
      ],
    });

    expect(screen.queryByText('No messages yet.')).not.toBeInTheDocument();
    expect(
      screen.getByText("329 encrypted messages can't be read on this device"),
    ).toBeInTheDocument();
    // 认识的用名字，最多点三个名，其余只说人数。
    expect(
      screen.getByText('From Ada, @builder:agent-room.test, @tester:agent-room.test and 1 other.'),
    ).toBeInTheDocument();
    expect(screen.getByText('This device never received their keys.')).toBeInTheDocument();
    expect(
      screen.getByText(
        "The sender didn't share the keys with this device because it wasn't signed yet.",
      ),
    ).toBeInTheDocument();
  });

  it('已经请 Agent 重发时说明收到后会自动解开，没在等时不说', () => {
    harness(
      false,
      [],
      null,
      { count: 3, reasons: ['missing_key'], senders: [agentId] },
      'requested',
    );
    expect(screen.getByText(/asked to send their keys again/u)).toBeInTheDocument();
    cleanup();
    harness(false, [], null, { count: 3, reasons: ['missing_key'], senders: [agentId] });
    expect(screen.queryByText(/asked to send their keys again/u)).not.toBeInTheDocument();
  });

  it('这台设备还没签好时说明会自动请重发，不给任何按钮', () => {
    harness(
      false,
      [],
      null,
      { count: 12, reasons: ['withheld'], senders: [agentId] },
      'awaiting_signing',
    );

    const notice = screen.getByText(/being signed now/u).closest('.conversation-undecryptable');
    expect(notice).toBeInstanceOf(HTMLElement);
    expect(within(notice as HTMLElement).queryByRole('button')).not.toBeInTheDocument();
    expect(screen.queryByText(/asked to send their keys again/u)).not.toBeInTheDocument();
  });

  it('一条也照样提示，发送者都点得出名时不说“其余”', () => {
    harness(false, [], null, { count: 1, reasons: ['other'], senders: [agentId] });

    expect(
      screen.getByText("1 encrypted message can't be read on this device"),
    ).toBeInTheDocument();
    expect(screen.getByText('From Ada.')).toBeInTheDocument();
    expect(screen.getByText('Decryption failed.')).toBeInTheDocument();
  });
});

describe('人与 Agent 直接聊天', () => {
  it.each([
    [
      'publication.encryption_not_ready',
      'isn’t ready for encrypted rooms yet',
      'Retry this message',
    ],
    ['publication.identity_changed', 'has a new encryption identity', 'Confirm and send'],
  ] as const)(
    '加密发送前被拦下 %s 时说明原因，保留草稿并用同一提交继续',
    async (code, guidance, action) => {
      const runtime = harness();
      runtime.publish.mockResolvedValueOnce(err({ code, retryable: true }));
      const user = userEvent.setup();
      const input = screen.getByRole('textbox', { name: 'Message' });
      await waitFor(() => {
        expect(input).toBeEnabled();
      });
      await user.type(input, 'Keep this encrypted draft.');
      await user.click(screen.getByRole('button', { name: 'Send' }));
      expect(await screen.findByRole('alert')).toHaveTextContent(guidance);
      expect(input).toHaveValue('Keep this encrypted draft.');
      expect(screen.queryByRole('button', { name: 'Check delivery' })).not.toBeInTheDocument();
      await user.click(screen.getByRole('button', { name: action }));
      await waitFor(() => {
        expect(input).toHaveValue('');
      });
      expect(runtime.publish).toHaveBeenCalledTimes(2);
      expect(runtime.publish.mock.calls[0]?.[0].submissionId).toBe(
        runtime.publish.mock.calls[1]?.[0].submissionId,
      );
    },
  );
  it('字数离上限还远时不显示计数，快满了才提醒；发送按钮是珊瑚色', async () => {
    harness();
    const input = screen.getByRole('textbox', { name: 'Message' });
    await waitFor(() => {
      expect(input).toBeEnabled();
    });
    fireEvent.change(input, { target: { value: 'Short question' } });
    expect(screen.queryByText(/\/ 4000$/u)).not.toBeInTheDocument();
    fireEvent.change(input, { target: { value: 'x'.repeat(3_700) } });
    expect(screen.getByText('3700 / 4000')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send' })).toHaveClass('ar-button--send');
  });
  it('键盘上 Enter 发送，触屏上 Enter 只换行、由按钮发送', async () => {
    const runtime = harness();
    const user = userEvent.setup();
    const input = screen.getByRole('textbox', { name: 'Message' });
    await waitFor(() => {
      expect(input).toBeEnabled();
    });
    // 输入框只收 4000 字符，与校验和计数一致，不再放进 8000 再报错。
    expect(input).toHaveAttribute('maxlength', '4000');
    await user.type(input, 'From a keyboard{Enter}');
    await waitFor(() => {
      expect(runtime.publish).toHaveBeenCalledOnce();
    });
    expect(runtime.publish.mock.calls[0]?.[0]).toMatchObject({
      conversation: { text: 'From a keyboard' },
    });

    // 软键盘没有 Shift 键：触屏上 Enter 换行，发送靠按钮。
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({ matches: query === '(pointer: coarse)', media: query })),
    );
    try {
      await waitFor(() => {
        expect(input).toHaveValue('');
      });
      await user.type(input, 'first line{Enter}second line');
      expect(input).toHaveValue('first line\nsecond line');
      expect(runtime.publish).toHaveBeenCalledOnce();
      await user.click(screen.getByRole('button', { name: 'Send' }));
      await waitFor(() => {
        expect(runtime.publish).toHaveBeenCalledTimes(2);
      });
      expect(runtime.publish.mock.calls[1]?.[0]).toMatchObject({
        conversation: { text: 'first line\nsecond line' },
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it('一段输入与稳定身份提及直接发布，并清空已发送草稿', async () => {
    const runtime = harness();
    const user = userEvent.setup();
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
    });
    await user.selectOptions(screen.getByRole('combobox', { name: 'Mention an agent' }), agentId);
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'What do you think?');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(runtime.publish).toHaveBeenCalledOnce();
    });
    expect(runtime.publish.mock.calls[0]?.[0]).toMatchObject({
      roomId,
      submissionId,
      mediaType: 'text/plain',
      conversation: { text: 'What do you think?', mentions: [agentId] },
    });
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('');
    });
  });

  it('私人房间的 @ 菜单里有“所有人”，发出去带上 @所有人；公开大厅里没有这一项', async () => {
    const publish = vi.fn((request: MessagePublicationRequest): Promise<MessagePublicationResult> =>
      Promise.resolve(
        ok({
          kind: 'published' as const,
          matrixEventId: '$sent',
          reused: false,
          submissionId: request.submissionId,
        }),
      ),
    );
    const publisher: MessagePublisher = {
      publish,
      reconcile: (id) =>
        Promise.resolve(
          ok({
            kind: 'published' as const,
            matrixEventId: '$sent',
            reused: true,
            submissionId: id,
          }),
        ),
      resolveIdentity: () =>
        Promise.resolve(
          ok({
            kind: 'human',
            displayName: 'Rainy',
            matrixUserId: '@rainy:agent-room.test',
            principalId: submissionId,
            source: 'matrix_human_session',
          }),
        ),
    };
    // 同一个发送编号来源：换一个会重建输入框的状态。
    const submissionIds = { next: () => submissionId };
    const view = (privateRoom: boolean) => (
      <I18nextProvider i18n={i18n}>
        <ConversationPanel
          messages={[]}
          publisher={publisher}
          roomId={roomId}
          state="ready"
          participants={[{ matrixUserId: agentId, displayName: 'Ada' }]}
          submissionIds={submissionIds}
          privateRoom={privateRoom}
        />
      </I18nextProvider>
    );
    const user = userEvent.setup();
    const { rerender } = render(view(false));
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
    });
    const menu = () => screen.getByRole('combobox', { name: 'Mention an agent' });
    expect(within(menu()).queryByRole('option', { name: 'Everyone' })).toBeNull();

    rerender(view(true));
    await user.selectOptions(menu(), 'Everyone');
    expect(screen.getByRole('button', { name: 'Remove @everyone' })).toBeInTheDocument();
    expect(within(menu()).queryByRole('option', { name: 'Everyone' })).toBeNull();
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'Please all take a look');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(publish).toHaveBeenCalledOnce();
    });
    expect(publish.mock.calls[0]?.[0]).toMatchObject({
      mentionsEveryone: true,
      conversation: { text: 'Please all take a look', mentions: [] },
    });
  });

  it('结果未知时锁住新发送，只使用原提交查询', async () => {
    const runtime = harness(true);
    const user = userEvent.setup();
    await waitFor(() => {
      expect(screen.getByRole('textbox', { name: 'Message' })).toBeEnabled();
    });
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'hello');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await screen.findByText('Delivery is being confirmed. Check before sending again.');
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: 'Check delivery' }));
    await waitFor(() => {
      expect(runtime.reconcile).toHaveBeenCalledWith(
        submissionId,
        runtime.publish.mock.calls[0]?.[0],
      );
    });
    expect(runtime.publish).toHaveBeenCalledOnce();
  });

  it('Agent 回复里的代码块按代码渲染，复制按钮复制原文', async () => {
    const text = ['改这里：', '```rust', 'let x = 1;', '```'].join('\n');
    const message: RoomMessageSignal = {
      actor: {
        agentId: submissionId,
        instanceId: submissionId,
        displayName: 'Ada',
        kind: 'agent',
        matrixUserId: agentId,
        provenance: 'human_confirmed_agent',
      },
      messageId: submissionId,
      matrixEventId: '$code',
      roomId,
      lifecycle: 'active',
      edited: false,
      endToEndEncrypted: false,
      serverTimestamp: 1_000,
      signatureStatus: 'instance_verified',
      content: null,
      preview: {
        title: 'Code',
        summary: 'Code',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text, mentions: [] },
      },
    };
    harness(false, [message]);
    const writeText = vi.fn(() => Promise.resolve());
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    try {
      const code = document.querySelector('pre.chat-markdown__code');
      expect(code).toHaveAttribute('data-language', 'rust');
      expect(code?.textContent).toBe('let x = 1;');
      // user-event 会换掉 navigator.clipboard，这里直接触发点击以验证写入的原文。
      fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
      expect(writeText).toHaveBeenCalledWith(text);
      expect(await screen.findByRole('button', { name: 'Copied' })).toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('网络 Agent 的消息头标出“网络 Agent”，别的 Agent 仍标“Agent”', async () => {
    const message = (id: string, agent: string): RoomMessageSignal => ({
      actor: {
        agentId: agent,
        instanceId: agent,
        displayName: 'Ada',
        kind: 'agent',
        matrixUserId: agentId,
        provenance: 'autonomous_agent',
      },
      messageId: id,
      matrixEventId: `$${id}`,
      roomId,
      lifecycle: 'active',
      edited: false,
      endToEndEncrypted: false,
      serverTimestamp: 1_000,
      signatureStatus: 'instance_verified',
      content: null,
      preview: {
        title: id,
        summary: id,
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: `Hello from ${id}`, mentions: [] },
      },
    });
    const network = '01990d9e-8400-7000-8000-000000000021';
    const local = '01990d9e-8400-7000-8000-000000000022';
    const labels = new NetworkAgentLabelStore(
      { lookup: (ids) => Promise.resolve(ok(new Set(ids.filter((id) => id === network)))) },
      {
        schedule: (task) => {
          task();
        },
      },
    );
    harness(false, [message('net', network), message('bridge', local)], labels);

    const header = (id: string) => {
      const article = document.querySelector(`[data-conversation-message-id="${id}"]`);
      const found = article?.querySelector('header');
      if (!(found instanceof HTMLElement)) throw new Error(`missing header for ${id}`);
      return found;
    };
    await waitFor(() => {
      expect(header('net')).toHaveTextContent('Network agent');
    });
    expect(header('bridge')).not.toHaveTextContent('Network agent');
    expect(header('bridge')).toHaveTextContent('Agent');
  });

  it('只有参与了回复关系的消息才显示「查看回复话题」', () => {
    const base: RoomMessageSignal = {
      actor: {
        agentId: submissionId,
        instanceId: submissionId,
        displayName: 'Ada',
        kind: 'agent',
        matrixUserId: agentId,
        provenance: 'human_confirmed_agent',
      },
      messageId: 'root',
      matrixEventId: '$root',
      roomId,
      lifecycle: 'active',
      edited: false,
      endToEndEncrypted: false,
      serverTimestamp: 1_000,
      signatureStatus: 'instance_verified',
      content: null,
      preview: {
        title: 'Root',
        summary: 'Root',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: 'Root question', mentions: [] },
      },
    };
    const reply: RoomMessageSignal = {
      ...base,
      messageId: 'reply',
      matrixEventId: '$reply',
      serverTimestamp: 2_000,
      relation: { kind: 'reply', targetMessageId: 'root' },
      preview: {
        title: 'Reply',
        summary: 'Reply',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: 'An answer', mentions: [] },
      },
    };
    const aside: RoomMessageSignal = {
      ...base,
      messageId: 'aside',
      matrixEventId: '$aside',
      serverTimestamp: 3_000,
    };
    harness(false, [base, reply, aside]);
    const links = screen.getAllByRole('button', { name: 'View this reply topic' });
    expect(
      links.map((link) =>
        link
          .closest('[data-conversation-message-id]')
          ?.getAttribute('data-conversation-message-id'),
      ),
    ).toEqual(['root', 'reply']);
  });

  describe('进房间自动补历史', () => {
    type LoadResult = Result<void, { readonly code: string; readonly retryable: boolean }>;
    const conversation = (index: number): RoomMessageSignal => ({
      actor: {
        agentId: submissionId,
        instanceId: submissionId,
        displayName: 'Ada',
        kind: 'agent',
        matrixUserId: agentId,
        provenance: 'human_confirmed_agent',
      },
      messageId: `message-${String(index)}`,
      matrixEventId: `$message-${String(index)}`,
      roomId,
      lifecycle: 'active',
      edited: false,
      endToEndEncrypted: false,
      serverTimestamp: 1_000 + index,
      signatureStatus: 'instance_verified',
      content: null,
      preview: {
        title: 'Chat',
        summary: 'Chat',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: `Message ${String(index)}`, mentions: [] },
      },
    });

    // jsdom 不排版：把时间线的可见高度和内容高度设成给定值。
    function layout(contentHeight: number): () => void {
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
          return sized(this, contentHeight);
        },
      });
      return () => {
        Reflect.deleteProperty(HTMLElement.prototype, 'clientHeight');
        Reflect.deleteProperty(HTMLElement.prototype, 'scrollHeight');
      };
    }

    function renderHistory(onLoadOlder: () => Promise<LoadResult>) {
      const publisher: MessagePublisher = {
        publish: vi.fn(),
        reconcile: vi.fn(),
        resolveIdentity: () =>
          Promise.resolve(err({ code: 'publication.identity_unavailable', retryable: true })),
      };
      render(
        <I18nextProvider i18n={i18n}>
          <ConversationPanel
            history={{ canLoadMore: true, limited: false }}
            onLoadOlder={onLoadOlder}
            messages={[conversation(1), conversation(2)]}
            publisher={publisher}
            roomId={roomId}
            state="ready"
          />
        </I18nextProvider>,
      );
    }

    it('撑不满一屏半时一次补一页，前一次没完不发下一次，每次打开最多三次', async () => {
      // 可见高度 500，内容 740，差一点到一屏半。
      const restore = layout(740);
      try {
        const pending: ((result: LoadResult) => void)[] = [];
        const onLoadOlder = vi.fn(
          () =>
            new Promise<LoadResult>((resolve) => {
              pending.push(resolve);
            }),
        );
        renderHistory(onLoadOlder);
        expect(onLoadOlder).toHaveBeenCalledOnce();
        expect(screen.getByRole('button', { name: 'Loading earlier messages…' })).toBeDisabled();
        for (const calls of [2, 3]) {
          await act(async () => {
            pending.shift()?.(ok(undefined));
            await Promise.resolve();
          });
          expect(onLoadOlder).toHaveBeenCalledTimes(calls);
        }
        await act(async () => {
          pending.shift()?.(ok(undefined));
          await Promise.resolve();
        });
        expect(onLoadOlder).toHaveBeenCalledTimes(3);
        expect(screen.getByRole('button', { name: 'Load earlier messages' })).toBeEnabled();
      } finally {
        restore();
      }
    });

    it('内容已经填满一屏半时不自动加载', () => {
      const restore = layout(750);
      try {
        const onLoadOlder = vi.fn(() => Promise.resolve<LoadResult>(ok(undefined)));
        renderHistory(onLoadOlder);
        expect(onLoadOlder).not.toHaveBeenCalled();
      } finally {
        restore();
      }
    });

    it('自动加载失败后不再自动重试，留给用户手动加载', async () => {
      const restore = layout(400);
      try {
        const onLoadOlder = vi.fn(() =>
          Promise.resolve<LoadResult>(err({ code: 'history.load_failed', retryable: true })),
        );
        renderHistory(onLoadOlder);
        expect(
          await screen.findByText(
            'Earlier messages could not be loaded. Your current conversation is still available.',
          ),
        ).toBeInTheDocument();
        expect(onLoadOlder).toHaveBeenCalledOnce();
      } finally {
        restore();
      }
    });
  });

  it('回复保留关联且远端 HTML 只显示为文字', async () => {
    const message: RoomMessageSignal = {
      actor: {
        agentId: submissionId,
        instanceId: submissionId,
        displayName: 'Ada',
        kind: 'agent',
        matrixUserId: agentId,
        provenance: 'human_confirmed_agent',
      },
      messageId: submissionId,
      matrixEventId: '$question',
      roomId,
      lifecycle: 'active',
      edited: false,
      endToEndEncrypted: false,
      serverTimestamp: 1_000,
      signatureStatus: 'instance_verified',
      content: null,
      preview: {
        title: 'Question',
        summary: 'Question',
        contentType: 'text/plain',
        riskFlags: [],
        sensitivity: 'normal',
        conversation: { text: '<img src=x onerror=alert(1)>', mentions: [] },
      },
    };
    const runtime = harness(false, [message]);
    const user = userEvent.setup();
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Reply to Ada' })).toBeEnabled();
    });
    await user.click(screen.getByRole('button', { name: 'Reply to Ada' }));
    await user.type(screen.getByRole('textbox', { name: 'Message' }), 'My answer');
    await user.click(screen.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(runtime.publish).toHaveBeenCalledOnce();
    });
    expect(runtime.publish.mock.calls[0]?.[0]).toMatchObject({
      relation: { kind: 'reply', targetMessageId: submissionId },
      conversation: { mentions: [agentId] },
    });
  });
});
