import type { LobbyAgentStatus } from '@/features/lobby/domain/lobby';
import { characterSeed } from '@/features/lobby/domain/room-floor';
import type { SceneShape } from './room-art';

export const characterStatusColor: Readonly<Record<LobbyAgentStatus | 'present', string>> = {
  present: '#446d9b',
  working: '#416944',
  idle: '#357c84',
  completed: '#54752c',
  waiting_input: '#986126',
  blocked: '#a34438',
  offline: '#797c72',
};

export function characterBodyArt(
  agentId: string,
  kind: 'agent' | 'human' = 'agent',
): readonly SceneShape[] {
  const seed = characterSeed(agentId);
  const outfit =
    kind === 'human'
      ? '#749bc5'
      : (['#8e9dcb', '#88b6a7', '#d3aa73', '#b58ba7', '#7facc4', '#d39079'][seed % 6] ?? '#8e9dcb');
  const skin = ['#f3d1b1', '#e8b895', '#b98361', '#dfad87'][seed % 4] ?? '#f3d1b1';
  const hair = ['#4b473d', '#785d49', '#50535d', '#9b7757'][seed % 4] ?? '#4b473d';
  return [
    {
      kind: 'rect',
      x: -15,
      y: -35,
      width: 30,
      height: 24,
      radius: 8,
      fill: outfit,
      stroke: '#566357',
    },
    { kind: 'rect', x: -4, y: -30, width: 8, height: 10, radius: 2, fill: '#f5efd9' },
    {
      kind: 'rect',
      x: -15,
      y: -61,
      width: 30,
      height: 29,
      radius: 12,
      fill: skin,
      stroke: '#866b50',
    },
    { kind: 'rect', x: -16, y: -63, width: 32, height: 13, radius: 7, fill: hair },
    { kind: 'rect', x: -16, y: -56, width: 6, height: 15, radius: 3, fill: hair },
    { kind: 'ellipse', x: -5, y: -45, rx: 2, ry: 2.5, fill: '#303b35' },
    { kind: 'ellipse', x: 7, y: -45, rx: 2, ry: 2.5, fill: '#303b35' },
    { kind: 'rect', x: -1, y: -37, width: 7, height: 2, radius: 1, fill: '#b27965' },
    {
      kind: 'rect',
      x: 13,
      y: -50,
      width: 5,
      height: 11,
      radius: 2,
      fill: '#eff0dc',
      stroke: '#798c77',
    },
    ...(seed % 3 === 0
      ? [
          {
            kind: 'rect' as const,
            x: -9,
            y: -49,
            width: 23,
            height: 9,
            radius: 3,
            fill: '#64888a',
            stroke: '#3d5c59',
          },
        ]
      : []),
  ];
}
