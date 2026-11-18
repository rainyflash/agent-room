// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  PrivateRoom,
  PrivateRoomGateway,
  PrivateRoomPermissions,
} from '@/features/private-rooms/domain/private-room';
import type {
  PrivateRoomInviteGateway,
  PrivateRoomInviteLink,
} from '@/features/private-rooms/domain/private-room-invite-link';
import { renderShareCard } from '@/features/room-share/ui/share-card-canvas';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';
import { roomTone } from '@/shared/ui/room-tone';

import { PrivateRoomInviteDialog } from './private-room-invite-dialog';

// jsdom 没有画布：卡片画成什么样在浏览器验收里查，这里查交给它画的是什么。
vi.mock('@/features/room-share/ui/share-card-canvas', () => ({
  renderShareCard: vi.fn(() => Promise.resolve(new Blob(['png'], { type: 'image/png' }))),
}));

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const INVITER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

const room: PrivateRoom = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: '',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    {
      permissions: { capabilities: ['view', 'speak', 'invite'] },
      principalId: INVITER,
      status: 'joined',
    },
  ],
  name: 'Weekly review',
  ownerPrincipalId: OWNER,
  retentionDays: null,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 1,
};

function linkWith(permissions: PrivateRoomPermissions, secret: string): PrivateRoomInviteLink {
  return {
    createdAtUnixMs: Date.UTC(2026, 9, 14),
    expiresAtUnixMs: Date.UTC(2026, 9, 21),
    permissions,
    secret,
    url: `https://agentroom.test/join#${secret}`,
  };
}

/** 和服务器一样记着一条链接：生成就换掉，停用就没了。 */
function invites(initial: PrivateRoomInviteLink | null = null, revealable = true) {
  let current = initial;
  let generated = 0;
  const gateway = {
    inviteLink: vi.fn<PrivateRoomInviteGateway['inviteLink']>(() =>
      Promise.resolve(
        current === null
          ? err({ code: 'invite_link.not_found', retryable: false })
          : revealable
            ? ok(current)
            : err({ code: 'invite_link.not_revealable', retryable: false }),
      ),
    ),
    createInviteLink: vi.fn<PrivateRoomInviteGateway['createInviteLink']>((_room, permissions) => {
      generated += 1;
      current = linkWith(
        permissions ?? { capabilities: ['view', 'speak', 'automate'] },
        `${String(generated).repeat(2)}3456789abcdefghjkmnpqrs`,
      );
      revealable = true;
      return Promise.resolve(ok(current));
    }),
    disableInviteLink: vi.fn<PrivateRoomInviteGateway['disableInviteLink']>(() => {
      current = null;
      return Promise.resolve(ok(undefined));
    }),
    previewInvite: vi.fn<PrivateRoomInviteGateway['previewInvite']>(),
    redeemInvite: vi.fn<PrivateRoomInviteGateway['redeemInvite']>(),
  };
  return gateway;
}

function renderDialog(
  gateway: PrivateRoomInviteGateway,
  principalId = OWNER,
  inviterName: string | undefined = 'Mina',
) {
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <PrivateRoomInviteDialog
          inviterName={inviterName}
          invites={gateway}
          onClose={() => undefined}
          principalId={principalId}
          room={room}
          rooms={{} as PrivateRoomGateway}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
  URL.createObjectURL = vi.fn(() => 'blob:card');
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  cleanup();
  vi.mocked(renderShareCard).mockClear();
});

describe('邀请人', () => {
  it('还没有链接时一个按钮生成，默认给查看、发言、自动发言', async () => {
    const gateway = invites();
    renderDialog(gateway);

    fireEvent.click(await screen.findByRole('button', { name: 'Make link and copy' }));

    expect(
      await screen.findByText('https://agentroom.test/join#113456789abcdefghjkmnpqrs'),
    ).toBeVisible();
    expect(gateway.createInviteLink).toHaveBeenCalledWith(room.catalogId, {
      capabilities: ['view', 'speak', 'automate'],
    });
    expect(screen.getByText(/Anyone with the link can come in after signing in/u)).toBeVisible();
  });

  it('有链接时旁边是邀请卡片：写着谁请的、哪间房、到哪天，二维码是这条链接', async () => {
    const link = linkWith(
      { capabilities: ['view', 'speak', 'automate'] },
      'abcdefghjkmnpqrs23456789',
    );
    renderDialog(invites(link));

    expect(
      await screen.findByRole('img', {
        name: 'Invite card for Weekly review, with a QR code for the link',
      }),
    ).toBeVisible();
    expect(renderShareCard).toHaveBeenCalledWith({
      action: 'Scan to join',
      lead: 'Mina invites you to a room',
      notes: ['Sign in and you’re in', expect.stringMatching(/^Valid until Oct 2[01]$/u)],
      roomName: 'Weekly review',
      tagline: 'Rooms where people and AI agents talk',
      tone: roomTone(room.catalogId),
      url: link.url,
    });
    expect(screen.getByText(/The QR code is this link\./u)).toBeVisible();
    // 多了一栏，对话框宽一档。
    expect(screen.getByRole('dialog', { name: 'Invite people' })).toHaveClass('ar-dialog--wide');
  });

  it('没有昵称时卡片上不写名字；还没有链接时没有卡片，对话框是平常的宽度', async () => {
    const gateway = invites();
    renderDialog(gateway, OWNER, '  ');

    expect(await screen.findByRole('button', { name: 'Make link and copy' })).toBeVisible();
    expect(screen.queryByRole('heading', { name: 'Invite card' })).toBeNull();
    expect(screen.getByRole('dialog', { name: 'Invite people' })).not.toHaveClass(
      'ar-dialog--wide',
    );

    fireEvent.click(screen.getByRole('button', { name: 'Make link and copy' }));

    await waitFor(() => {
      expect(renderShareCard).toHaveBeenCalledWith(
        expect.objectContaining({ lead: 'You’re invited to a room' }),
      );
    });
  });

  it('一起发的话带着链接，能改；换了一条，话跟着换成新的链接', async () => {
    const gateway = invites(
      linkWith({ capabilities: ['view', 'speak', 'automate'] }, 'abcdefghjkmnpqrs23456789'),
    );
    renderDialog(gateway);

    const message = await screen.findByLabelText('What you’ll say');
    expect(message).toHaveValue(
      'Join me in “Weekly review” on Agent Room. Open the link and sign in:\nhttps://agentroom.test/join#abcdefghjkmnpqrs23456789',
    );
    fireEvent.change(message, { target: { value: 'Come by tonight' } });
    expect(message).toHaveValue('Come by tonight');

    fireEvent.click(screen.getByRole('button', { name: 'New link' }));

    await waitFor(() => {
      expect(screen.getByLabelText('What you’ll say')).toHaveValue(
        'Join me in “Weekly review” on Agent Room. Open the link and sign in:\nhttps://agentroom.test/join#113456789abcdefghjkmnpqrs',
      );
    });
    // 卡片也照新的链接重画。
    expect(renderShareCard).toHaveBeenLastCalledWith(
      expect.objectContaining({ url: 'https://agentroom.test/join#113456789abcdefghjkmnpqrs' }),
    );
  });

  it('不是房主的给不出自己没有的：默认权限按自己有的截一下', async () => {
    const gateway = invites();
    renderDialog(gateway, INVITER);

    fireEvent.click(await screen.findByRole('button', { name: 'Make link and copy' }));

    await waitFor(() => {
      expect(gateway.createInviteLink).toHaveBeenCalledWith(room.catalogId, {
        capabilities: ['view', 'speak'],
      });
    });
    const legend = screen.getByRole('group', { name: 'Link permissions' });
    expect(within(legend).getByRole('checkbox', { name: /^Automate/u })).toBeDisabled();
  });

  it('改了权限才出现“按这些权限换一条”，换完回到跟着链接走', async () => {
    const gateway = invites(
      linkWith({ capabilities: ['view', 'speak', 'automate'] }, '0'.repeat(26)),
    );
    renderDialog(gateway);
    expect(await screen.findByText(`https://agentroom.test/join#${'0'.repeat(26)}`)).toBeVisible();
    expect(screen.queryByRole('button', { name: 'New link with these' })).toBeNull();

    fireEvent.click(
      within(screen.getByRole('group', { name: 'Link permissions' })).getByRole('checkbox', {
        name: /^Automate/u,
      }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'New link with these' }));

    await waitFor(() => {
      expect(gateway.createInviteLink).toHaveBeenCalledWith(room.catalogId, {
        capabilities: ['view', 'speak'],
      });
    });
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'New link with these' })).toBeNull();
    });
  });

  it('链接给不出“管理”，房主也一样；以前建的带着“管理”的，给出换一条，换了就不带', async () => {
    const gateway = invites(
      linkWith({ capabilities: ['view', 'speak', 'manage', 'automate'] }, '0'.repeat(26)),
    );
    renderDialog(gateway);
    expect(await screen.findByText(`https://agentroom.test/join#${'0'.repeat(26)}`)).toBeVisible();
    const manage = within(screen.getByRole('group', { name: 'Link permissions' })).getByRole(
      'checkbox',
      { name: /^Manage/u },
    );
    expect(manage).not.toBeChecked();
    expect(manage).toBeDisabled();

    fireEvent.click(screen.getByRole('button', { name: 'New link with these' }));
    await waitFor(() => {
      expect(gateway.createInviteLink).toHaveBeenCalledWith(room.catalogId, {
        capabilities: ['view', 'speak', 'automate'],
      });
    });
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'New link with these' })).toBeNull();
    });
  });

  it('看不到现在那条时让换一条；停用以后谁都用不了', async () => {
    const gateway = invites(linkWith({ capabilities: ['view'] }, '0'.repeat(26)), false);
    renderDialog(gateway);
    expect(await screen.findByText(/can’t be shown here/u)).toBeVisible();

    fireEvent.click(screen.getByRole('button', { name: 'Make link and copy' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Turn off link' }));

    expect(await screen.findByText('The link is off. Nobody can use it now.')).toBeVisible();
    expect(gateway.disableInviteLink).toHaveBeenCalledWith(room.catalogId);
  });

  it('按账户 ID 邀请收在折叠里，默认也给查看、发言、自动发言', async () => {
    renderDialog(invites());
    await screen.findByRole('button', { name: 'Make link and copy' });
    const form = within(screen.getByRole('group', { name: 'Invitation permissions' }));
    expect(form.getByRole('checkbox', { name: /^Speak/u })).toBeChecked();
    expect(form.getByRole('checkbox', { name: /^Automate/u })).toBeChecked();
    expect(form.getByRole('checkbox', { name: /^Invite/u })).not.toBeChecked();
    expect(screen.getByLabelText('Account ID')).toBeInTheDocument();
  });

  it('服务器说不能邀请时只说一句，不给按钮', async () => {
    const gateway = invites();
    gateway.inviteLink.mockResolvedValue(err({ code: 'invite_link.forbidden', retryable: false }));
    renderDialog(gateway);
    expect(
      await screen.findByText('Only members who can invite others can make a link.'),
    ).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Make link and copy' })).toBeNull();
  });
});
