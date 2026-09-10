// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  readAutomationGrantDraft,
  saveAutomationGrantDraft,
  type AutomationGrantDraftInput,
} from './automation-grant-draft';
import { err, ok } from '@/shared/result';

const owner = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const draft: AutomationGrantDraftInput = {
  agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  agentInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e45',
  roomCatalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  audience: 'any_room_member',
  lifetimeSeconds: 3600,
  maxMessagesPerMinute: 3,
  maxTotalMessages: 20,
  messageKinds: ['reply'],
  requiresRiskScan: true,
};

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  window.sessionStorage.clear();
});

describe('Automation grant drafts', () => {
  it('草稿最多恢复一小时，时钟倒退或换账户也不恢复', () => {
    vi.useFakeTimers();
    const now = Date.now();
    expect(saveAutomationGrantDraft(owner, draft)).toEqual(ok(undefined));
    expect(readAutomationGrantDraft(draft.agentId)).toEqual(ok(null));
    vi.setSystemTime(now - 1);
    expect(readAutomationGrantDraft(owner)).toEqual(ok(null));
    vi.setSystemTime(now + 3600_001);
    expect(readAutomationGrantDraft(owner)).toEqual(ok(null));
  });

  it('存储被禁用时报告失败，不能声称已保存', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('blocked', 'SecurityError');
    });
    expect(saveAutomationGrantDraft(owner, draft)).toEqual(err('storage'));
  });

  it('已损坏的草稿不进入表单', () => {
    expect(saveAutomationGrantDraft(owner, draft)).toEqual(ok(undefined));
    const key = window.sessionStorage.key(0);
    expect(key).not.toBeNull();
    if (key === null) throw new Error('Expected the saved draft');
    window.sessionStorage.setItem(key, '{');
    expect(readAutomationGrantDraft(owner)).toEqual(err('storage'));
  });
});
