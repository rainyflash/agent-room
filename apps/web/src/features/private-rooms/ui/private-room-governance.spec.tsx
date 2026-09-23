// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import type { PrivateRoom, PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok } from '@/shared/result';

import { PrivateRoomGovernance } from './private-room-governance';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

const room: PrivateRoom = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  description: 'Private review',
  matrixRoomId: '!private:matrix.test',
  members: [
    {
      permissions: { capabilities: ['view', 'speak', 'invite', 'manage', 'automate'] },
      principalId: OWNER,
      status: 'joined',
    },
    {
      permissions: { capabilities: ['view', 'speak'] },
      principalId: MEMBER,
      status: 'joined',
    },
  ],
  name: 'Architecture room',
  ownerPrincipalId: OWNER,
  retentionDays: 30,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
  status: 'active',
  version: 2,
};

function gateway(): { value: PrivateRoomGateway; rename: ReturnType<typeof vi.fn> } {
  const unavailable = () => Promise.resolve(err({ code: 'test.unavailable', retryable: false }));
  const rename = vi.fn((_catalogId: string, name: string) =>
    Promise.resolve(ok({ ...room, name })),
  );
  return {
    rename,
    value: {
      accept: unavailable,
      archive: unavailable,
      ban: unavailable,
      create: unavailable,
      decline: unavailable,
      inspect: unavailable,
      invite: unavailable,
      leave: unavailable,
      list: () => Promise.resolve(ok([])),
      remove: unavailable,
      rename,
      transferOwnership: unavailable,
      updatePermissions: unavailable,
    },
  };
}

function renderGovernance(principalId: string, rooms: PrivateRoomGateway) {
  return render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={new QueryClient()}>
        <PrivateRoomGovernance
          coordinator={{} as PrivateRoomCoordinator}
          onExitRoom={() => undefined}
          principalId={principalId}
          recentlyAuthenticated={false}
          room={room}
          rooms={rooms}
        />
      </QueryClientProvider>
    </I18nextProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en-US']);
});

afterEach(() => {
  cleanup();
});

describe('PrivateRoomGovernance', () => {
  it('房主可以改名：名字没变或为空时不能提交', async () => {
    const rooms = gateway();
    renderGovernance(OWNER, rooms.value);
    const input = screen.getByLabelText('Room name');
    const button = screen.getByRole('button', { name: 'Rename' });
    expect(input).toHaveValue('Architecture room');
    expect(button).toBeDisabled();
    fireEvent.change(input, { target: { value: '   ' } });
    expect(button).toBeDisabled();
    fireEvent.change(input, { target: { value: '  Design review ' } });
    expect(button).toBeEnabled();
    fireEvent.click(button);
    await waitFor(() => {
      expect(rooms.rename).toHaveBeenCalledWith(room.catalogId, 'Design review');
    });
  });

  it('成员只看到房间名，不能改', () => {
    const rooms = gateway();
    renderGovernance(MEMBER, rooms.value);
    expect(screen.getByText('Architecture room')).toBeVisible();
    expect(screen.queryByLabelText('Room name')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Rename' })).toBeNull();
  });
});
