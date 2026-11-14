// @vitest-environment jsdom

import '@testing-library/jest-dom/vitest';

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type { MessageActor } from '@/features/messages/domain/message';

import type { LobbyAgent } from '../domain/lobby';
import { characterVariant } from '../scene/studio-assets';
import { ActorAvatar, AgentAvatar, HumanAvatar, LobbyAgentAvatar } from './avatars';

afterEach(cleanup);

const avatar = (): HTMLElement => {
  const element = document.querySelector('.ar-avatar');
  if (!(element instanceof HTMLElement)) throw new Error('avatar not rendered');
  return element;
};

/** 找一个落在指定那一列机器人上的 ID。 */
function idWithVariant(variant: number): string {
  for (let index = 0; index < 200; index += 1) {
    const id = `agent-${String(index)}`;
    if (characterVariant(id) === variant) return id;
  }
  throw new Error('no id for variant');
}

describe('Agent 的头像', () => {
  it('垫的浅底跟着它是哪一个机器人：青、红、黄，带翅膀的也是青', () => {
    for (const [variant, tint] of [
      [0, 'teal'],
      [1, 'coral'],
      [2, 'sunny'],
      [3, 'teal'],
    ] as const) {
      render(<AgentAvatar id={idWithVariant(variant)} />);
      expect(avatar()).toHaveClass(`ar-avatar--${tint}`);
      expect(avatar().querySelector('svg.agent-portrait')).not.toBeNull();
      cleanup();
    }
  });

  it('在等消息是实心的点，叫得醒是圈，连着没在等不画点，离线整个变灰', () => {
    const { rerender } = render(<AgentAvatar id="a" state="waiting" stateLabel="Waiting" />);
    expect(screen.getByRole('img', { name: 'Waiting' })).toHaveClass('ar-presence-dot--waiting');

    rerender(<AgentAvatar id="a" state="wakes" stateLabel="Can be woken" />);
    expect(screen.getByRole('img', { name: 'Can be woken' })).toHaveClass('ar-presence-dot--wakes');

    rerender(<AgentAvatar id="a" state="idle" />);
    expect(avatar().querySelector('.ar-presence-dot')).toBeNull();
    expect(avatar()).not.toHaveClass('ar-avatar--dimmed');

    rerender(<AgentAvatar id="a" state="offline" />);
    expect(avatar().querySelector('.ar-presence-dot')).toBeNull();
    expect(avatar()).toHaveClass('ar-avatar--dimmed');
  });

  it('旁边已经用字写了状态时，点不再读一遍', () => {
    render(<AgentAvatar id="a" state="waiting" />);
    expect(screen.queryByRole('img')).toBeNull();
    expect(avatar().querySelector('.ar-presence-dot')).toHaveAttribute('aria-hidden', 'true');
  });

  it('房间里的 Agent 按它此刻的状态画', () => {
    const agent: LobbyAgent = {
      agentId: 'agent',
      displayName: 'Mira',
      instanceIds: ['instance'],
      matrixUserId: '@mira:studio.test',
      status: 'idle',
      statusExpiresAtUnixMs: 100_000,
      trust: 'unknown',
      visibility: 'coarse',
      lastPolledAtUnixMs: 10_000,
      listeningUntilUnixMs: 25_000,
    };
    const { rerender } = render(<LobbyAgentAvatar agent={agent} now={20_000} />);
    expect(avatar().querySelector('.ar-presence-dot--waiting')).not.toBeNull();

    rerender(<LobbyAgentAvatar agent={{ ...agent, status: 'offline' }} now={90_000} />);
    expect(avatar()).toHaveClass('ar-avatar--dimmed');
  });

  it('叫得醒的画成圈：离线的也不变灰；正在等消息的照旧是实心的点', () => {
    const agent: LobbyAgent = {
      agentId: 'agent',
      displayName: 'Mira',
      instanceIds: ['instance'],
      matrixUserId: '@mira:studio.test',
      status: 'offline',
      statusExpiresAtUnixMs: 100_000,
      trust: 'unknown',
      visibility: 'coarse',
    };
    const { rerender } = render(<LobbyAgentAvatar agent={agent} now={90_000} wakeable />);
    expect(avatar().querySelector('.ar-presence-dot--wakes')).not.toBeNull();
    expect(avatar()).not.toHaveClass('ar-avatar--dimmed');

    rerender(
      <LobbyAgentAvatar
        agent={{
          ...agent,
          status: 'idle',
          lastPolledAtUnixMs: 10_000,
          listeningUntilUnixMs: 25_000,
        }}
        now={20_000}
        wakeable
      />,
    );
    expect(avatar().querySelector('.ar-presence-dot--waiting')).not.toBeNull();
    expect(avatar().querySelector('.ar-presence-dot--wakes')).toBeNull();
  });
});

describe('谁发的', () => {
  const agent: MessageActor = {
    agentId: 'agent',
    displayName: 'Ada',
    instanceId: 'instance',
    kind: 'agent',
    matrixUserId: '@ada:agent-room.test',
    provenance: 'autonomous_agent',
  };
  const human: MessageActor = {
    displayName: 'rainy',
    kind: 'human',
    matrixUserId: '@rainy:agent-room.test',
    principalId: 'principal',
    provenance: 'human',
  };

  it('Agent 画机器人，人画墨色的圆、里面是名字的第一个字', () => {
    const { rerender } = render(<ActorAvatar actor={agent} />);
    expect(avatar().querySelector('svg.agent-portrait')).not.toBeNull();
    expect(avatar()).not.toHaveClass('ar-avatar--ink');

    rerender(<ActorAvatar actor={human} />);
    expect(avatar()).toHaveClass('ar-avatar--ink');
    expect(avatar()).toHaveTextContent('R');
    expect(avatar().querySelector('svg')).toBeNull();
  });

  it('头像只是画，读屏不读它', () => {
    render(<HumanAvatar name="小王" />);
    expect(avatar().querySelector('.ar-avatar__face')).toHaveAttribute('aria-hidden', 'true');
  });
});
