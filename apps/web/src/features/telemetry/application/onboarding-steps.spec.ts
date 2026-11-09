import { describe, expect, it, vi } from 'vitest';

import type { OnboardingStepGateway } from '@/features/telemetry/domain/onboarding-step';
import { reportOnboardingStep } from './onboarding-steps';

function memoryStorage(): Pick<Storage, 'getItem' | 'setItem'> {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

function gateway() {
  const recordStep = vi.fn<OnboardingStepGateway['recordStep']>().mockResolvedValue(undefined);
  return { recordStep };
}

describe('reportOnboardingStep', () => {
  it('同一个账户每一步只报一次，复制按方式各算一次', () => {
    const steps = gateway();
    const storage = memoryStorage();

    for (let attempt = 0; attempt < 3; attempt += 1) {
      reportOnboardingStep(steps, storage, 'account-1', 'web', { step: 'invite_opened' });
      reportOnboardingStep(steps, storage, 'account-1', 'web', {
        step: 'invite_copied',
        method: 'cli',
      });
    }
    reportOnboardingStep(steps, storage, 'account-1', 'web', {
      step: 'invite_copied',
      method: 'network',
    });

    expect(steps.recordStep.mock.calls.map(([sample]) => sample)).toEqual([
      { step: 'invite_opened', surface: 'web' },
      { step: 'invite_copied', method: 'cli', surface: 'web' },
      { step: 'invite_copied', method: 'network', surface: 'web' },
    ]);
  });

  it('换了账户重新算，发出去的内容里没有账户', () => {
    const steps = gateway();
    const storage = memoryStorage();

    reportOnboardingStep(steps, storage, 'account-1', 'desktop', { step: 'invite_opened' });
    reportOnboardingStep(steps, storage, 'account-2', 'desktop', { step: 'invite_opened' });

    expect(steps.recordStep).toHaveBeenCalledTimes(2);
    expect(JSON.stringify(steps.recordStep.mock.calls)).not.toContain('account-');
  });

  it('本机存不了就不报', () => {
    const steps = gateway();
    const blocked = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => undefined,
    };

    reportOnboardingStep(steps, null, 'account-1', 'web', { step: 'invite_opened' });
    reportOnboardingStep(steps, blocked, 'account-1', 'web', { step: 'invite_opened' });

    expect(steps.recordStep).not.toHaveBeenCalled();
  });
});
