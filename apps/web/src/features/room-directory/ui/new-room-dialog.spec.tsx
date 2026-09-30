// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  CreatePrivateRoomInput,
  PrivateRoom,
  PrivateRoomFailure,
} from '@/features/private-rooms/domain/private-room';
import { NewRoomForm } from '@/features/room-directory/ui/new-room-dialog';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok, type Result } from '@/shared/result';

const first = '0198b601-77a2-7f41-b4f4-940f291951b8';
const second = '0198b601-77a2-7f41-b4f4-940f291951b9';

const created: PrivateRoom = {
  catalogId: '0198b601-77a2-7f41-b4f4-940f29195100',
  description: '',
  matrixRoomId: '!release:matrix.test',
  members: [],
  name: 'Release workshop',
  ownerPrincipalId: first,
  retentionDays: null,
  roomInstanceId: '0198b601-77a2-7f41-b4f4-940f29195101',
  status: 'active',
  version: 1,
};

type CreateCall = (
  requestId: string,
  input: CreatePrivateRoomInput,
) => Promise<Result<PrivateRoom, PrivateRoomFailure>>;

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(async () => {
  window.localStorage.clear();
  await i18n.changeLanguage('en');
});

afterEach(cleanup);

function renderForm(onCreate: CreateCall = vi.fn<CreateCall>(() => Promise.resolve(ok(created)))) {
  const onClose = vi.fn();
  render(
    <I18nextProvider i18n={i18n}>
      <NewRoomForm onClose={onClose} onCreate={onCreate} />
    </I18nextProvider>,
  );
  return { onClose };
}

function submit(): HTMLElement {
  const button = screen.getByRole('button', { name: /Create and enter|Try again/ });
  fireEvent.click(button);
  return button;
}

describe('新建房间', () => {
  it('一屏：只填名字就能建，用途可空，保留天数和邀请都用默认值', async () => {
    const onCreate = vi.fn<CreateCall>(() => Promise.resolve(ok(created)));
    const { onClose } = renderForm(onCreate);

    expect(screen.getByRole('dialog', { name: 'New room' })).toHaveAccessibleDescription(
      'Private rooms are invite only, and messages are end-to-end encrypted.',
    );
    expect(screen.getByRole('button', { name: 'Create and enter' })).toBeDisabled();
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: '  Release workshop ' },
    });
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(onCreate).toHaveBeenCalledWith(expect.any(String), {
      description: '',
      invitations: [],
      name: 'Release workshop',
    });
  });

  it('更多选项里可以定保留多久、现在就邀请谁、他们能做什么', async () => {
    const onCreate = vi.fn<CreateCall>(() => Promise.resolve(ok(created)));
    renderForm(onCreate);

    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Design review' },
    });
    fireEvent.change(screen.getByRole('textbox', { name: 'Purpose (optional)' }), {
      target: { value: 'Review the new lobby.' },
    });
    fireEvent.click(screen.getByText('More options'));
    fireEvent.change(screen.getByRole('combobox', { name: 'Keep messages for' }), {
      target: { value: '30' },
    });
    expect(screen.queryByRole('group', { name: 'What they can do' })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole('textbox', { name: 'Invite people now (optional)' }), {
      target: { value: `${first}\n${second}` },
    });
    fireEvent.click(screen.getByRole('checkbox', { name: /Automate/ }));
    expect(screen.getByRole('checkbox', { name: /Speak/ })).toBeChecked();
    submit();

    await waitFor(() => {
      expect(onCreate).toHaveBeenCalledOnce();
    });
    const permissions = { capabilities: ['view', 'speak', 'automate'] };
    expect(onCreate).toHaveBeenCalledWith(expect.any(String), {
      description: 'Review the new lobby.',
      invitations: [
        { permissions, principalId: first },
        { permissions, principalId: second },
      ],
      name: 'Design review',
      retentionDays: 30,
    });
  });

  it('邀请里有不是账户 ID 的内容时说明原因，先不让建', () => {
    renderForm();

    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Design review' },
    });
    const invites = screen.getByRole('textbox', { name: 'Invite people now (optional)' });
    fireEvent.change(invites, { target: { value: 'alice@example.com' } });

    expect(invites).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByRole('alert')).toHaveTextContent('Each line must be an account ID.');
    expect(screen.getByRole('button', { name: 'Create and enter' })).toBeDisabled();
  });

  it('没建成时说重试不会再建一个；重试沿用同一个请求，填的内容锁住', async () => {
    const onCreate = vi
      .fn<CreateCall>()
      .mockResolvedValueOnce(err({ code: 'private_room.matrix_join_failed', retryable: true }))
      .mockResolvedValueOnce(ok(created));
    const { onClose } = renderForm(onCreate);

    const name = screen.getByRole('textbox', { name: 'Name' });
    fireEvent.change(name, { target: { value: 'Release workshop' } });
    submit();

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Could not finish creating the room. Try again; it will not create a second one.',
    );
    expect(screen.getByRole('alert')).toHaveTextContent('private_room.matrix_join_failed');
    expect(name).toBeDisabled();
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(onCreate).toHaveBeenCalledTimes(2);
    const [firstCall, secondCall] = onCreate.mock.calls;
    expect(secondCall).toEqual(firstCall);
  });
});
