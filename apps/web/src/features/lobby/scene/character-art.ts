import type { LobbyAgentStatus } from '@/features/lobby/domain/lobby';

export const characterStatusColor: Readonly<Record<LobbyAgentStatus | 'present', string>> = {
  present: '#446d9b',
  working: '#416944',
  idle: '#357c84',
  completed: '#54752c',
  waiting_input: '#986126',
  blocked: '#a34438',
  offline: '#797c72',
};
