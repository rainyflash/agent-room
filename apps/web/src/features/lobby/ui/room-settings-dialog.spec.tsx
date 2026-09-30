// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import type { WebSession } from '@/features/session/domain/session';
import { RoomSettingsDialog } from '@/features/lobby/ui/room-settings-dialog';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

const services = vi.hoisted((): { current: Record<string, unknown> } => ({ current: {} }));
vi.mock('@/app/app-services', () => ({
  useAppServices: () => services.current,
  useOptionalAppServices: () => services.current,
}));

const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

const principal = (principalId: string): WebSession => ({
  authenticatedAtUnixMs: 1,
  displayName: 'Mina',
  expiresAtUnixMs: 9_999_999_999_999,
  locale: 'en',
  matrixUserId: '@mina:matrix.test',
  principalId,
  recentlyAuthenticated: true,
});

const privateRoom: PrivateRoom = {
  catalogId: CATALOG,
  description: 'Private review',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    { permissions: { capabilities: ['view', 'speak'] }, principalId: MEMBER, status: 'joined' },
  ],
  name: 'Architecture room',
  ownerPrincipalId: OWNER,
  retentionDays: null,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 2,
};

function setServices({
  rooms,
  moderator,
}: {
  readonly rooms: readonly PrivateRoom[];
  readonly moderator: boolean;
}) {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  services.current = {
    accessManagement: { listAgentInstances: () => Promise.resolve(ok([])) },
    automation: { create: unavailable, list: () => Promise.resolve(ok([])), revoke: unavailable },
    config: { controlPlaneUrl: 'https://agent-room.test' },
    controlPlane: { beginAuthentication: vi.fn() },
    localRuntime: { isAvailable: () => false },
    moderation: {
      inspectCapabilities: () =>
        Promise.resolve(ok({ canModerateRoom: moderator, canReadAudit: moderator })),
      listActions: () => Promise.resolve(ok([])),
      listAudit: () => Promise.resolve(ok([])),
      listRoomCases: () => Promise.resolve(ok([])),
    },
    privateRoomMatrix: { join: unavailable, leave: unavailable },
    privateRooms: {
      agentAccess: () => Promise.resolve(ok({ agents: [], joinCode: null })),
      list: () => Promise.resolve(ok(rooms)),
    },
  };
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderDialog(principalId: string, initialSection?: 'automation') {
  const onClose = vi.fn();
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <RoomSettingsDialog
          catalogId={CATALOG}
          initialSection={initialSection}
          onClose={onClose}
          onExitRoom={vi.fn()}
          principal={principal(principalId)}
          roomName="Architecture room"
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
  return { onClose };
}

describe('房间设置', () => {
  it('公共大厅里的普通成员只有自动发言一节，不显示分节切换', async () => {
    setServices({ moderator: false, rooms: [] });
    renderDialog(MEMBER);

    const dialog = screen.getByRole('dialog', { name: 'Room settings' });
    expect(dialog).toHaveAccessibleDescription('Architecture room');
    expect(await within(dialog).findByRole('region', { name: 'Automation' })).toBeVisible();
    expect(within(dialog).queryByRole('radiogroup')).not.toBeInTheDocument();
  });

  it('私人房间的房主：成员、Agent 口令、自动发言、治理，一层切换', async () => {
    setServices({ moderator: true, rooms: [privateRoom] });
    renderDialog(OWNER);

    const sections = await screen.findByRole('radiogroup', { name: 'Room settings sections' });
    expect(
      within(sections)
        .getAllByRole('radio')
        .map((option) => option.textContent),
    ).toEqual(['Members', 'Agent code', 'Automation', 'Moderation']);
    expect(within(sections).getByRole('radio', { name: 'Members' })).toBeChecked();
    expect(screen.getByLabelText('Room name')).toHaveValue('Architecture room');
  });

  it('切到别的节再切回来，填到一半的内容还在', async () => {
    setServices({ moderator: false, rooms: [privateRoom] });
    renderDialog(OWNER);

    const invitee = await screen.findByRole('textbox', { name: 'Account ID' });
    fireEvent.change(invitee, { target: { value: '0198b601-77a1-7bb8' } });
    fireEvent.click(screen.getByRole('radio', { name: 'Automation' }));
    expect(screen.getByRole('region', { name: 'Automation' })).toBeVisible();
    expect(screen.queryByRole('region', { name: 'Members' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('radio', { name: 'Members' }));
    expect(screen.getByRole('textbox', { name: 'Account ID' })).toHaveValue('0198b601-77a1-7bb8');
  });

  it('不能管理房间的成员没有 Agent 口令一节', async () => {
    setServices({ moderator: false, rooms: [privateRoom] });
    renderDialog(MEMBER);

    const sections = await screen.findByRole('radiogroup', { name: 'Room settings sections' });
    expect(
      within(sections)
        .getAllByRole('radio')
        .map((option) => option.textContent),
    ).toEqual(['Members', 'Automation']);
  });

  it('重新登录回来有没提交完的自动发言授权时，直接打开那一节', async () => {
    setServices({ moderator: true, rooms: [privateRoom] });
    renderDialog(OWNER, 'automation');

    const sections = await screen.findByRole('radiogroup', { name: 'Room settings sections' });
    expect(within(sections).getByRole('radio', { name: 'Automation' })).toBeChecked();
  });
});
