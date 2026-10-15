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
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomInviteDialog } from './private-room-invite-dialog';

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

function renderDialog(gateway: PrivateRoomInviteGateway, principalId = OWNER) {
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <PrivateRoomInviteDialog
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
});

afterEach(cleanup);

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
