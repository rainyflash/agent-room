// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';
import type { MessagePreview, RoomMessageSignal } from '@/features/messages/domain/message';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

import type { ExcerptShareState } from './excerpt-selection';
import { ExcerptSelectionBar } from './excerpt-selection-bar';
import { ShareExcerptDialog } from './share-excerpt-dialog';

const URL = 'https://agentroom.test/r/aB3xYz/s/Kq7Lm2Zp';
const AT = new Date(2026, 10, 16, 9, 30).getTime();

function said(
  id: string,
  preview: Partial<MessagePreview> = {},
  extra: Partial<RoomMessageSignal> = {},
) {
  return conversationFixture(id, {
    serverTimestamp: AT,
    preview: {
      title: 'Conversation',
      summary: id,
      contentType: 'text/plain',
      riskFlags: [],
      sensitivity: 'normal',
      conversation: { text: `Line ${id}`, mentions: [] },
      ...preview,
    },
    ...extra,
  });
}

const MESSAGES: readonly RoomMessageSignal[] = [
  said('a'),
  said('b', { sensitivity: 'sensitive' }),
  said('c', { conversation: { text: 'plan.pdf', attachmentName: 'plan.pdf', mentions: [] } }),
  said('d', {}, { edited: true }),
];

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderDialog(
  state: ExcerptShareState,
  handlers: { onRetry?: () => void; onClose?: () => void } = {},
) {
  render(
    <I18nextProvider i18n={i18n}>
      <ShareExcerptDialog
        messages={MESSAGES}
        onClose={handlers.onClose ?? vi.fn()}
        onRetry={handlers.onRetry ?? vi.fn()}
        roomName="Rust help desk"
        state={state}
      />
    </I18nextProvider>,
  );
  return screen.getByRole('dialog', { name: 'Share this stretch' });
}

describe('分享这一段的对话框', () => {
  it('还在生成链接时先说一声，没有链接和二维码；预览已经在了', () => {
    const dialog = renderDialog({ kind: 'pending' });
    expect(within(dialog).getByRole('status')).toHaveTextContent('Making the link…');
    expect(within(dialog).queryByRole('textbox', { name: 'Link' })).toBeNull();
    expect(within(dialog).queryByRole('img')).toBeNull();
    expect(within(dialog).getByRole('list', { name: /^4 messages · / })).toBeVisible();
  });

  it('生成好了：链接、复制、发到别处、二维码；预览照分享出去的那一页', () => {
    const dialog = renderDialog({ kind: 'shared', url: URL });
    expect(within(dialog).getByRole('textbox', { name: 'Link' })).toHaveValue(URL);
    expect(within(dialog).getByRole('button', { name: 'Copy link' })).toBeVisible();
    const x = new globalThis.URL(
      within(dialog)
        .getByRole('link', { name: 'Post to X (opens in a new tab)' })
        .getAttribute('href') ?? '',
    );
    expect(x.searchParams.get('url')).toBe(URL);
    expect(x.searchParams.get('text')).toBe('A conversation from “Rust help desk”');
    expect(
      within(dialog).getByRole('img', { name: 'QR code for the link to this stretch' }),
    ).toBeVisible();

    const lines = within(within(dialog).getByRole('list', { name: /^4 messages · / })).getAllByRole(
      'listitem',
    );
    expect(lines.map((line) => line.textContent)).toEqual([
      expect.stringContaining('Line a'),
      expect.stringContaining('A message that isn’t shown publicly'),
      expect.stringContaining('Sent a file'),
      expect.stringContaining('Line d (edited)'),
    ]);
    expect(lines[1]).not.toHaveTextContent('Line b');
    expect(lines[2]).not.toHaveTextContent('plan.pdf');
    expect(lines[0]).toHaveTextContent('Alex');
    expect(lines[0]).toHaveTextContent('Person');
  });

  it('没成：说为什么，错误码只在详情里；选的不对给“改选”，连不上给“再试一次”', () => {
    const onClose = vi.fn();
    const before: PrivateRoomFailure = {
      code: 'room_excerpt.invalid_range',
      reason: 'before_live',
      correlationId: 'corr-1',
      retryable: false,
    };
    let dialog = renderDialog({ kind: 'failed', failure: before }, { onClose });
    const alert = within(dialog).getByRole('alert');
    expect(alert).toHaveTextContent('Couldn’t share this stretch');
    expect(alert).toHaveTextContent(
      'Only messages said after the live view was turned on can be shared. Start from a later message.',
    );
    expect(within(alert).getByText('room_excerpt.invalid_range')).not.toBeVisible();
    expect(within(dialog).queryByRole('textbox', { name: 'Link' })).toBeNull();
    expect(within(dialog).queryByRole('button', { name: 'Try again' })).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Change the selection' }));
    expect(onClose).toHaveBeenCalledOnce();
    cleanup();

    const onRetry = vi.fn();
    dialog = renderDialog(
      { kind: 'failed', failure: { code: 'private_room.unreachable', retryable: true } },
      { onRetry },
    );
    expect(within(dialog).getByRole('alert')).toHaveTextContent(
      'Can’t reach Agent Room. Check your connection and try again.',
    );
    fireEvent.click(within(dialog).getByRole('button', { name: 'Try again' }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});

describe('选的时候对话上面那一条', () => {
  function renderBar(props: {
    count: number;
    tooLong?: boolean;
    sharing?: boolean;
    label?: string;
  }) {
    const onShare = vi.fn();
    const onCancel = vi.fn();
    render(
      <I18nextProvider i18n={i18n}>
        <ExcerptSelectionBar
          count={props.count}
          onCancel={onCancel}
          onShare={onShare}
          sharing={props.sharing ?? false}
          tooLong={props.tooLong ?? false}
        />
      </I18nextProvider>,
    );
    return {
      bar: screen.getByRole('region', { name: props.label ?? 'Share a stretch of conversation' }),
      onShare,
      onCancel,
    };
  }

  it('写选了几句、怎么接着选；读屏会念出来', () => {
    const { bar, onShare, onCancel } = renderBar({ count: 6 });
    expect(within(bar).getByRole('status')).toHaveTextContent(
      '6 selectedPick a later message to end the stretch there, up to 30 messages.',
    );
    fireEvent.click(within(bar).getByRole('button', { name: 'Share' }));
    fireEvent.click(within(bar).getByRole('button', { name: 'Cancel' }));
    expect(onShare).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it('够不着时说最多 30 句；正在要链接时“分享”先关着', () => {
    const { bar } = renderBar({ count: 1, tooLong: true, sharing: true });
    expect(within(bar).getByRole('status')).toHaveTextContent(
      'A stretch can be at most 30 messages. Pick a message closer to the start.',
    );
    expect(within(bar).getByRole('button', { name: 'Share' })).toBeDisabled();
  });

  it('中文：已选 6 句', async () => {
    await i18n.changeLanguage('zh-CN');
    const { bar } = renderBar({ count: 6, label: '分享一段对话' });
    expect(within(bar).getByRole('status')).toHaveTextContent('已选 6 句');
    expect(within(bar).getByRole('button', { name: '分享' })).toBeVisible();
    expect(within(bar).getByRole('button', { name: '取消' })).toBeVisible();
  });
});
