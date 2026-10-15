import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import { permissions } from '@/features/private-rooms/domain/private-room';
import type { PrivateRoomInvitePreview } from '@/features/private-rooms/domain/private-room-invite-link';
import {
  JoinInviteView,
  type JoinInviteState,
} from '@/features/room-directory/ui/join-invite-page';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

/**
 * 打开邀请链接那一页（`/join`）的各种样子：
 * - 默认：Ada 邀请你进“Weekly review”，给查看、发言、自动发言；`?speak` 只给查看和发言；`?unnamed` 邀请人没有昵称；
 * - `?already` 你已经在房间里；
 * - `?expired` 链接过期或停用，`?banned` 你被封禁了，`?missing` 链接不完整，`?busy` 试得太多，`?failed` 别的错。
 * 点“加入”以后停在“正在加入”，`window.__agentRoomJoinClicks` 记着点了几次。
 */
const search = new URLSearchParams(window.location.search);

const fixtureWindow = window as Window & { __agentRoomJoinClicks?: number };

const failures = {
  expired: 'invite_link.not_found',
  banned: 'invite_link.banned',
  missing: 'invite_link.invalid',
  busy: 'invite_link.rate_limited',
  failed: 'invite_link.dependency_unavailable',
} as const;

function initialState(): JoinInviteState {
  for (const [flag, code] of Object.entries(failures)) {
    if (search.has(flag)) {
      return {
        kind: 'failure',
        failure: { code, retryable: flag === 'busy' || flag === 'failed' },
      };
    }
  }
  if (search.has('loading')) return { kind: 'loading' };
  const preview: PrivateRoomInvitePreview = {
    catalogId: '0198b601-77a4-7f41-b4f4-940f291951c0',
    expiresAtUnixMs: Date.UTC(2026, 9, 21, 9, 30),
    inviterDisplayName: search.has('unnamed') ? '' : 'Ada',
    membership: search.has('already') ? 'joined' : 'absent',
    permissions: search.has('speak')
      ? permissions('view', 'speak')
      : permissions('view', 'speak', 'automate'),
    roomName: 'Weekly review',
  };
  return { kind: 'preview', joining: false, preview };
}

function JoinInviteFixture() {
  const [state, setState] = useState(initialState);
  return (
    <JoinInviteView
      onJoin={() => {
        fixtureWindow.__agentRoomJoinClicks = (fixtureWindow.__agentRoomJoinClicks ?? 0) + 1;
        setState((current) =>
          current.kind === 'preview' ? { ...current, joining: true } : current,
        );
      }}
      onRetry={() => {
        setState({ kind: 'loading' });
      }}
      state={state}
    />
  );
}

async function bootstrapFixture(): Promise<void> {
  await initializeI18n(window.localStorage, ['en']);
  const root = document.querySelector('#root');
  if (!(root instanceof HTMLElement)) {
    throw new Error('邀请链接测试根节点不存在。');
  }
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      <JoinInviteFixture />
    </I18nextProvider>,
  );
}

void bootstrapFixture();
