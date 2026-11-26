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
import type { RepositoryClaim } from '@/features/private-rooms/domain/project-rooms';
import { NewRoomForm } from '@/features/room-directory/ui/new-room-dialog';
import {
  PROJECT_CLAIM_POLL_MS,
  type ProjectRoomSource,
} from '@/features/room-directory/ui/new-project-room';
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

type EnterCall = (room: PrivateRoom) => Promise<void>;
type LiveViewCall = (catalogId: string) => Promise<Result<unknown, PrivateRoomFailure>>;

function renderForm(
  onCreate: CreateCall = vi.fn<CreateCall>(() => Promise.resolve(ok(created))),
  initiallyListed = false,
  onTurnOnLiveView?: LiveViewCall,
  projectRoom?: ProjectRoomSource,
) {
  const onClose = vi.fn();
  const onEnter = vi.fn<EnterCall>(() => Promise.resolve());
  render(
    <I18nextProvider i18n={i18n}>
      <NewRoomForm
        initiallyListed={initiallyListed}
        onClose={onClose}
        onCreate={onCreate}
        onEnter={onEnter}
        onTurnOnLiveView={onTurnOnLiveView}
        projectRoom={projectRoom}
      />
    </I18nextProvider>,
  );
  return { onClose, onEnter };
}

function submit(): HTMLElement {
  const button = screen.getByRole('button', { name: /Create and enter|Try again/ });
  fireEvent.click(button);
  return button;
}

describe('新建房间', () => {
  it('一屏：只填名字就能建，用途可空，保留天数和邀请都用默认值', async () => {
    const onCreate = vi.fn<CreateCall>(() => Promise.resolve(ok(created)));
    const { onClose, onEnter } = renderForm(onCreate);

    expect(screen.getByRole('dialog', { name: 'New room' })).toHaveAccessibleDescription(
      'Private rooms are invite only and end-to-end encrypted. Public rooms are listed on Explore and not encrypted.',
    );
    expect(screen.getByRole('button', { name: 'Create and enter' })).toBeDisabled();
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: '  Release workshop ' },
    });
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(onEnter).toHaveBeenCalledWith(created);
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

  it('服务器说这样填不行时放开表单，改好以后按新请求再建', async () => {
    const onCreate = vi
      .fn<CreateCall>()
      .mockResolvedValueOnce(err({ code: 'private_room.invalid_request', retryable: false }))
      .mockResolvedValueOnce(ok(created));
    const { onClose } = renderForm(onCreate);

    const name = screen.getByRole('textbox', { name: 'Name' });
    fireEvent.change(name, { target: { value: 'Release workshop' } });
    submit();

    expect(await screen.findByRole('alert')).toHaveTextContent(
      "A room can't be created with these details.",
    );
    expect(name).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Create and enter' })).toBeEnabled();
    fireEvent.change(name, { target: { value: 'Release room' } });
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    const [firstCall, secondCall] = onCreate.mock.calls;
    expect(secondCall?.[0]).not.toBe(firstCall?.[0]);
    expect(secondCall?.[1]).toMatchObject({ name: 'Release room' });
  });

  it('默认私密、不问怎么进；选公开先说不加密，带上加入方式和默认的新成员权限', async () => {
    const onCreate = vi.fn<CreateCall>(() => Promise.resolve(ok(created)));
    renderForm(onCreate);

    expect(screen.getByRole('radio', { name: /^Private/u })).toBeChecked();
    expect(screen.queryByRole('group', { name: 'How people join' })).not.toBeInTheDocument();
    expect(screen.queryByText(/Public rooms are not encrypted/u)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('radio', { name: /^Public/u }));
    expect(screen.getByText(/Public rooms are not encrypted/u)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /^Join directly/u })).toBeChecked();
    fireEvent.click(screen.getByRole('radio', { name: /^Ask to join/u }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Eval club' },
    });
    submit();

    await waitFor(() => {
      expect(onCreate).toHaveBeenCalledWith(expect.any(String), {
        description: '',
        invitations: [],
        listing: {
          allowNetworkAgents: true,
          joinMode: 'request',
          listed: true,
          memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
        },
        name: 'Eval club',
      });
    });
  });

  it('从发现页打开时一开始就选着公开，照样能改回私密', () => {
    renderForm(undefined, true);

    expect(screen.getByRole('radio', { name: /^Public/u })).toBeChecked();
    expect(screen.getByText(/Public rooms are not encrypted/u)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('radio', { name: /^Private/u }));
    expect(screen.queryByRole('group', { name: 'How people join' })).not.toBeInTheDocument();
  });

  it('已经有一间公开房间：说清楚，放开表单，改成私密就能建', async () => {
    const onCreate = vi
      .fn<CreateCall>()
      .mockResolvedValueOnce(err({ code: 'private_room.public_limit_reached', retryable: false }))
      .mockResolvedValueOnce(ok(created));
    const { onClose } = renderForm(onCreate);

    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Eval club' },
    });
    fireEvent.click(screen.getByRole('radio', { name: /^Public/u }));
    submit();

    expect(
      await screen.findByText(
        'Each account can have only one public room that isn’t linked to a repository. Choose Private, or make your other one private first.',
      ),
    ).toBeInTheDocument();
    const privateChoice = screen.getByRole('radio', { name: /^Private/u });
    expect(privateChoice).toBeEnabled();
    fireEvent.click(privateChoice);
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(onCreate.mock.calls[1]?.[1]).not.toHaveProperty('listing');
  });
});

describe('新建时就公开的房间：分享页放实况', () => {
  const liveViewBox = () =>
    screen.queryByRole('checkbox', { name: /^Live view on the share page/u });

  function fillPublicRoom(): void {
    fireEvent.change(screen.getByRole('textbox', { name: 'Name' }), {
      target: { value: 'Eval club' },
    });
    fireEvent.click(screen.getByRole('radio', { name: /^Public/u }));
  }

  it('公开、直接进的房间才有这个勾，默认勾上；私密、要申请、只能邀请的没有', () => {
    renderForm(undefined, false, vi.fn<LiveViewCall>());

    expect(liveViewBox()).not.toBeInTheDocument();
    fillPublicRoom();
    expect(liveViewBox()).toBeChecked();
    expect(liveViewBox()).toHaveAccessibleName(
      /People who aren’t signed in can see what’s said here on the share page \(the latest 20 messages\)\. You can turn it off later in room settings, under Public\.$/u,
    );
    fireEvent.click(screen.getByRole('radio', { name: /^Ask to join/u }));
    expect(liveViewBox()).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('radio', { name: /^Invite only/u }));
    expect(liveViewBox()).not.toBeInTheDocument();
  });

  it('没有实况这项服务时不出勾', () => {
    renderForm(undefined, true);
    expect(liveViewBox()).not.toBeInTheDocument();
  });

  it('勾着：建好以后再开实况，建房的请求里不带它，开好了再进房间', async () => {
    const order: string[] = [];
    const onCreate = vi.fn<CreateCall>(() => {
      order.push('create');
      return Promise.resolve(ok(created));
    });
    const turnOn = vi.fn<LiveViewCall>(() => {
      order.push('live');
      return Promise.resolve(ok(undefined));
    });
    const { onClose, onEnter } = renderForm(onCreate, false, turnOn);

    fillPublicRoom();
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(turnOn).toHaveBeenCalledWith(created.catalogId);
    expect(order).toEqual(['create', 'live']);
    expect(onEnter).toHaveBeenCalledWith(created);
    // 建房的请求一个字段都不多：实况建好以后单独开。
    expect(onCreate.mock.calls[0]?.[1]).toEqual({
      description: '',
      invitations: [],
      listing: {
        allowNetworkAgents: true,
        joinMode: 'open',
        listed: true,
        memberPermissions: { capabilities: ['view', 'speak', 'automate'] },
      },
      name: 'Eval club',
    });
  });

  it('不勾就不开', async () => {
    const turnOn = vi.fn<LiveViewCall>(() => Promise.resolve(ok(undefined)));
    const { onClose } = renderForm(undefined, false, turnOn);

    fillPublicRoom();
    fireEvent.click(screen.getByRole('checkbox', { name: /^Live view on the share page/u }));
    expect(liveViewBox()).not.toBeChecked();
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(turnOn).not.toHaveBeenCalled();
  });

  it('实况没开成：房间照样在，说一句能在设置里开，按钮换成“进房间”，不再建', async () => {
    const onCreate = vi.fn<CreateCall>(() => Promise.resolve(ok(created)));
    const turnOn = vi.fn<LiveViewCall>(() =>
      Promise.resolve(err({ code: 'room_live.dependency_unavailable', retryable: true })),
    );
    const { onClose, onEnter } = renderForm(onCreate, false, turnOn);

    fillPublicRoom();
    submit();

    expect(
      await screen.findByText(
        'The room is ready, but the live view on the share page didn’t turn on. You can turn it on in room settings, under Public.',
      ),
    ).toBeInTheDocument();
    expect(screen.getByText('room_live.dependency_unavailable')).toBeInTheDocument();
    expect(onEnter).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('textbox', { name: 'Name' })).toBeDisabled();

    fireEvent.click(screen.getByRole('button', { name: 'Enter the room' }));

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(onEnter).toHaveBeenCalledWith(created);
    expect(onCreate).toHaveBeenCalledOnce();
    expect(turnOn).toHaveBeenCalledOnce();
  });

  it('房间没建成就不去开实况；重试建好以后再开', async () => {
    const onCreate = vi
      .fn<CreateCall>()
      .mockResolvedValueOnce(err({ code: 'private_room.matrix_join_failed', retryable: true }))
      .mockResolvedValueOnce(ok(created));
    const turnOn = vi.fn<LiveViewCall>(() => Promise.resolve(ok(undefined)));
    const { onClose } = renderForm(onCreate, false, turnOn);

    fillPublicRoom();
    submit();
    expect(await screen.findByRole('alert')).toHaveTextContent('private_room.matrix_join_failed');
    expect(turnOn).not.toHaveBeenCalled();
    // 第一次提交以后请求定下来了，勾也锁住。
    expect(liveViewBox()).toBeDisabled();
    submit();

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledOnce();
    });
    expect(turnOn).toHaveBeenCalledOnce();
  });

  describe('给一个 GitHub 项目建房', () => {
    const projectRoomName = /^A room for a GitHub project/u;
    const projectRoomBox = () => screen.queryByRole('checkbox', { name: projectRoomName });
    const chooseProjectRoom = () => {
      fireEvent.click(screen.getByRole('checkbox', { name: projectRoomName }));
    };

    function waitingClaim(repository: string, catalogId: string | null = null): RepositoryClaim {
      return {
        agentsLine: `Agent Room: https://agentroom.test/r/aB3xYz`,
        badgeMarkdown: '[![Agent Room](b)](https://agentroom.test/r/aB3xYz)',
        catalogId,
        claimId: '0198b601-77a2-7f41-b4f4-940f29195190',
        claimedAtUnixMs: 1_000,
        lastCheck: { atUnixMs: 2_000, result: 'not_in_repository' },
        repository,
        shareCode: 'aB3xYz',
        shareUrl: 'https://agentroom.test/r/aB3xYz',
        status: 'pending',
      };
    }

    function source(overrides: Partial<ProjectRoomSource> = {}): ProjectRoomSource {
      return {
        claims: vi.fn(() => Promise.resolve(ok([]))),
        findRoom: vi.fn(() => Promise.resolve(null)),
        invocation: 'agent-room',
        locate: true,
        ...overrides,
      };
    }

    afterEach(() => {
      vi.useRealTimers();
    });

    it('只在选公开时有；勾上以后名字、用途、怎么进、实况和更多选项都收起来，换成发给 Agent 的话', () => {
      renderForm(undefined, false, vi.fn<LiveViewCall>(), source());

      expect(projectRoomBox()).not.toBeInTheDocument();
      fireEvent.click(screen.getByRole('radio', { name: /^Public/u }));
      const box = screen.getByRole('checkbox', { name: projectRoomName });
      expect(box).not.toBeChecked();
      expect(box).toHaveAccessibleName(/Named after the repository/u);
      chooseProjectRoom();

      expect(screen.queryByRole('textbox', { name: 'Name' })).not.toBeInTheDocument();
      expect(screen.queryByRole('textbox', { name: 'Purpose' })).not.toBeInTheDocument();
      expect(screen.queryByRole('group', { name: 'How people join' })).not.toBeInTheDocument();
      expect(liveViewBox()).not.toBeInTheDocument();
      expect(screen.queryByText('More options')).not.toBeInTheDocument();
      expect(screen.queryByText(/Public rooms are not encrypted/u)).not.toBeInTheDocument();
      // 房间由服务器认上以后建：没有“建好进去”，明黄的只有复制。
      expect(screen.queryByRole('button', { name: /Create and enter/u })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Copy message' })).toHaveClass(
        'ar-button--primary',
      );
      const message = screen.getByText(/Help me give this project a room/u).textContent;
      expect(message).toContain('\nagent-room project\n');
      expect(message).toContain('run agent-room project --wait');
      expect(message).toContain('agent_room_claim_repository');
      expect(message).toContain('don’t commit or push until I say');
      expect(message).toMatch(/%LOCALAPPDATA%\\Agent Room\\agent-room\.exe/u);
      expect(screen.getByText('Only public repositories on github.com.')).toBeInTheDocument();
      expect(screen.getByText(/The room is public and not encrypted/u)).toBeInTheDocument();
      expect(screen.getByText(/doesn’t count toward your one public room/u)).toBeInTheDocument();

      // 改回私密：照常建一间普通的房。
      fireEvent.click(screen.getByRole('radio', { name: /^Private/u }));
      expect(projectRoomBox()).not.toBeInTheDocument();
      expect(screen.getByRole('textbox', { name: 'Name' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /Create and enter/u })).toBeInTheDocument();
    });

    it('桌面端写实际安装路径，不说去哪找', () => {
      renderForm(
        undefined,
        true,
        undefined,
        source({ invocation: "& 'C:\\Agent Room\\agent-room.exe'", locate: false }),
      );
      chooseProjectRoom();
      const message = screen.getByText(/Help me give this project a room/u).textContent;
      expect(message).toContain("& 'C:\\Agent Room\\agent-room.exe' project --wait");
      expect(message).not.toContain('%LOCALAPPDATA%');
    });

    it('列出在等的认领；一个没了、找到了同名的房，就给“进房间”', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const ready: PrivateRoom = { ...created, name: 'octo/hello' };
      const claims = vi
        .fn<ProjectRoomSource['claims']>()
        .mockResolvedValueOnce(
          ok([waitingClaim('octo/hello'), waitingClaim('octo/bound-elsewhere', created.catalogId)]),
        )
        .mockResolvedValue(ok([]));
      const findRoom = vi.fn<ProjectRoomSource['findRoom']>(() => Promise.resolve(ready));
      const { onClose, onEnter } = renderForm(
        undefined,
        true,
        undefined,
        source({ claims, findRoom }),
      );
      chooseProjectRoom();

      const waiting = await screen.findByRole('heading', { name: 'Waiting for these' });
      expect(waiting.parentElement).toHaveTextContent(
        /octo\/hello: the line isn’t in the repository yet/u,
      );
      // 绑已有房间的认领不在这里。
      expect(screen.queryByText('octo/bound-elsewhere')).not.toBeInTheDocument();
      expect(findRoom).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(PROJECT_CLAIM_POLL_MS);
      expect(await screen.findByRole('status')).toHaveTextContent(
        'The room for octo/hello is ready.',
      );
      expect(findRoom).toHaveBeenCalledWith('octo/hello');
      expect(screen.queryByRole('heading', { name: 'Waiting for these' })).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Copy message' })).toHaveClass('ar-button--ghost');

      fireEvent.click(screen.getByRole('button', { name: 'Enter the room' }));
      await waitFor(() => {
        expect(onClose).toHaveBeenCalledOnce();
      });
      expect(onEnter).toHaveBeenCalledWith(ready);
    });

    it('认领没了却找不到房间（过期、撤掉了）：只是从列表里消失', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const claims = vi
        .fn<ProjectRoomSource['claims']>()
        .mockResolvedValueOnce(ok([waitingClaim('octo/hello')]))
        .mockResolvedValue(ok([]));
      const findRoom = vi.fn<ProjectRoomSource['findRoom']>(() => Promise.resolve(null));
      renderForm(undefined, true, undefined, source({ claims, findRoom }));
      chooseProjectRoom();
      await screen.findByRole('heading', { name: 'Waiting for these' });

      await vi.advanceTimersByTimeAsync(PROJECT_CLAIM_POLL_MS);
      await waitFor(() => {
        expect(findRoom).toHaveBeenCalledWith('octo/hello');
      });
      expect(screen.queryByRole('heading', { name: 'Waiting for these' })).not.toBeInTheDocument();
      expect(screen.queryByRole('status')).not.toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Enter the room' })).not.toBeInTheDocument();
    });

    it('没有认领这项服务时不给这个选项', () => {
      renderForm(undefined, true);
      expect(projectRoomBox()).not.toBeInTheDocument();
    });
  });
});
