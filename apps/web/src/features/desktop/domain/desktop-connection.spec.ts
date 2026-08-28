import { describe, expect, it } from 'vitest';

import { desktopConnectionView } from '@/features/desktop/domain/desktop-connection';

describe('桌面连接投影', () => {
  it('设备授权后把 Agent 身份标为当前阶段', () => {
    const view = desktopConnectionView('authorized', false, false);
    expect(view.currentStage).toBe(2);
    expect(view.stages.map((stage) => stage.status)).toEqual([
      'complete',
      'complete',
      'current',
      'pending',
    ]);
  });

  it('Bridge 就绪后只展示全部完成的真实状态', () => {
    const view = desktopConnectionView('ready', false, false);
    expect(view.stages.every((stage) => stage.status === 'complete')).toBe(true);
    expect(view.tone).toBe('active');
  });

  it('稳定故障不会投影成进行中或成功', () => {
    const view = desktopConnectionView('halted', false, true);
    expect(view.stages[0]?.status).toBe('blocked');
    expect(view.tone).toBe('alert');
  });
});
