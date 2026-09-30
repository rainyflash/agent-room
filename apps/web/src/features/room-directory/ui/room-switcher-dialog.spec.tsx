// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import {
  RoomSwitcherView,
  type RoomSwitcherViewProps,
} from '@/features/room-directory/ui/room-switcher-dialog';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

vi.mock('@tanstack/react-router', () => ({
  Link: ({
    children,
    className,
    onClick,
    'aria-current': current,
  }: {
    readonly children: ReactNode;
    readonly className?: string;
    readonly onClick?: () => void;
    readonly 'aria-current'?: 'page';
  }) => (
    <a aria-current={current} className={className} href="#room" onClick={onClick}>
      {children}
    </a>
  ),
}));

const me = '0198b601-77a2-7f41-b4f4-940f29195000';

function privateRoom(name: string, status: 'invited' | 'joined', suffix: string): PrivateRoom {
  return {
    catalogId: `0198b601-77a2-7f41-b4f4-940f291951${suffix}`,
    description: `${name} purpose`,
    matrixRoomId: `!${suffix}:matrix.test`,
    members: [{ permissions: { capabilities: ['view', 'speak'] }, principalId: me, status }],
    name,
    ownerPrincipalId: '0198b601-77a2-7f41-b4f4-940f29195999',
    retentionDays: null,
    roomInstanceId: `0198b601-77a2-7f41-b4f4-940f291952${suffix}`,
    status: 'active',
    version: 1,
  };
}

const invitation = privateRoom('Research lab', 'invited', '01');
const studio = privateRoom('Design studio', 'joined', '02');
const engineering = privateRoom('Engineering room', 'joined', '03');
const lobby: PublicRoomSummary = {
  activeInstanceCount: 1,
  catalogId: '0198b601-77a2-7f41-b4f4-940f29195300',
  description: 'Meet people and agents',
  language: 'en',
  name: 'Global lobby',
  onlineAgentCount: 8,
  slug: 'global',
};

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderView(overrides: Partial<RoomSwitcherViewProps> = {}) {
  const props: RoomSwitcherViewProps = {
    currentCatalogId: studio.catalogId,
    failure: null,
    invitations: [invitation],
    joined: [studio, engineering],
    onClose: vi.fn(),
    onCreate: vi.fn(),
    onDecline: vi.fn(),
    onEnter: vi.fn(),
    onRetryPrivate: vi.fn(),
    onRetryPublic: vi.fn(),
    pendingCatalogId: null,
    principalId: me,
    privateState: 'ready',
    publicRooms: [lobby],
    publicState: 'ready',
    ...overrides,
  };
  render(
    <I18nextProvider i18n={i18n}>
      <RoomSwitcherView {...props} />
    </I18nextProvider>,
  );
  return props;
}

function section(name: string): HTMLElement {
  return screen.getByRole('region', { name });
}

describe('换个房间', () => {
  it('邀请在最上面，可以加入或拒绝', () => {
    const props = renderView();

    const invitations = section('Invitations');
    expect(invitations).toHaveTextContent('Research lab');
    fireEvent.click(within(invitations).getByRole('button', { name: 'Join' }));
    expect(props.onEnter).toHaveBeenCalledWith(invitation, true);
    fireEvent.click(within(invitations).getByRole('button', { name: 'Decline' }));
    expect(props.onDecline).toHaveBeenCalledWith(invitation);
  });

  it('你的私人房间：当前的标着“你在这里”，点别的就进去', () => {
    const props = renderView();

    const mine = section('Your private rooms');
    const current = within(mine).getByRole('button', { name: /Design studio/ });
    expect(current).toHaveAttribute('aria-current', 'page');
    expect(current).toHaveTextContent('You are here');
    expect(current).toBeDisabled();
    fireEvent.click(within(mine).getByRole('button', { name: /Engineering room/ }));
    expect(props.onEnter).toHaveBeenCalledWith(engineering, false);
  });

  it('公共大厅是链接；点了就关掉对话框', () => {
    const props = renderView();

    fireEvent.click(within(section('Public lobbies')).getByRole('link', { name: /Global lobby/ }));
    expect(props.onClose).toHaveBeenCalledOnce();
  });

  it('一个搜索框过滤全部；没有匹配时说一声', () => {
    renderView();

    fireEvent.change(screen.getByRole('searchbox', { name: 'Search rooms' }), {
      target: { value: 'engineering' },
    });
    expect(screen.queryByRole('region', { name: 'Invitations' })).not.toBeInTheDocument();
    expect(section('Your private rooms')).toHaveTextContent('Engineering room');
    expect(section('Your private rooms')).not.toHaveTextContent('Design studio');
    expect(section('Public lobbies')).toHaveTextContent('No rooms match.');
  });

  it('读取中、读取失败（能重试）和还没有私人房间时各说一句', () => {
    const props = renderView({
      invitations: [],
      joined: [],
      privateState: 'failed',
      publicState: 'loading',
    });

    fireEvent.click(
      within(section('Your private rooms')).getByRole('button', { name: 'Try again' }),
    );
    expect(props.onRetryPrivate).toHaveBeenCalledOnce();
    expect(section('Public lobbies')).toHaveTextContent('Loading rooms…');
    cleanup();

    renderView({ invitations: [], joined: [] });
    expect(section('Your private rooms')).toHaveTextContent(
      'No private rooms yet. Create one, or ask a room owner to invite you.',
    );
  });

  it('进行中别的操作先停着；没成功时一句话，错误码收进详情', () => {
    renderView({
      failure: 'private_room.matrix_join_failed',
      pendingCatalogId: invitation.catalogId,
    });

    expect(within(section('Invitations')).getByRole('button', { name: 'Decline' })).toBeDisabled();
    expect(
      within(section('Your private rooms')).getByRole('button', { name: /Engineering/ }),
    ).toBeDisabled();
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('That did not go through. Try again.');
    expect(within(alert).getByText('Details')).toBeInTheDocument();
    expect(alert).toHaveTextContent('private_room.matrix_join_failed');
  });

  it('底部新建房间；账户 ID 收在详情里，可以复制给房主', () => {
    const props = renderView();

    fireEvent.click(screen.getByRole('button', { name: 'New room' }));
    expect(props.onCreate).toHaveBeenCalledOnce();
    expect(screen.getByText('Your account ID')).toBeInTheDocument();
    expect(screen.getByLabelText('Your account ID')).toHaveTextContent(me);
  });
});
