import { describe, expect, it } from 'vitest';

import { err, ok } from '@/shared/result';

import type { ReleaseUpdateCheck, ReleaseUpdateStatus } from './desktop-runtime';
import { newerUpdateStatus, updateStatusAfterCheck } from './update-status';

const found: ReleaseUpdateCheck = {
  available: true,
  channel: 'testing',
  currentVersion: '0.1.0-alpha.67',
  rollback: false,
  sequence: 68,
  targetVersion: '0.1.0-alpha.68',
};

describe('更新检查的结果', () => {
  it('查成了换成这次的结果，没查成时留着上一次查成的', () => {
    const first = updateStatusAfterCheck(null, 'testing', ok(found), 10);
    expect(first).toEqual({ channel: 'testing', check: found, checkedAtUnixMs: 10, failure: null });

    const offline = updateStatusAfterCheck(
      first,
      'testing',
      err({ code: 'desktop.update.manifest_network', retryable: true }),
      20,
    );
    expect(offline).toEqual({
      channel: 'testing',
      check: found,
      checkedAtUnixMs: 20,
      failure: { code: 'desktop.update.manifest_network', retryable: true },
    });

    expect(
      updateStatusAfterCheck(
        null,
        'stable',
        err({ code: 'desktop.update.manifest_expired', retryable: false }),
        30,
      ).check,
    ).toBeNull();
  });

  it('快照和事件谁先到都留时间新的那份', () => {
    const older: ReleaseUpdateStatus = {
      channel: 'testing',
      check: null,
      checkedAtUnixMs: 10,
      failure: { code: 'desktop.update.manifest_network', retryable: true },
    };
    const newer: ReleaseUpdateStatus = {
      ...older,
      check: found,
      checkedAtUnixMs: 20,
      failure: null,
    };
    expect(newerUpdateStatus(null, older)).toBe(older);
    expect(newerUpdateStatus(older, null)).toBe(older);
    expect(newerUpdateStatus(older, newer)).toBe(newer);
    expect(newerUpdateStatus(newer, older)).toBe(newer);
  });
});
