import type {
  OnboardingStep,
  OnboardingStepGateway,
} from '@/features/telemetry/domain/onboarding-step';

const storagePrefix = 'agent-room.onboarding-steps.v1:';

function stepKey(step: OnboardingStep): string {
  return 'method' in step ? `${step.step}:${step.method}` : step.step;
}

/**
 * 同一个浏览器里，每个账户每一步只报一次，这样数到的大致是人数。“报过了”只记在本机、不发出去；
 * 本机存不了就不报：宁可少算，也别让一个人反复算进去。
 */
export function reportOnboardingStep(
  gateway: OnboardingStepGateway,
  storage: Pick<Storage, 'getItem' | 'setItem'> | null,
  accountId: string,
  surface: 'desktop' | 'web',
  step: OnboardingStep,
): void {
  if (storage === null) return;
  const key = `${storagePrefix}${accountId}`;
  const name = stepKey(step);
  try {
    const reported = (storage.getItem(key) ?? '').split(',').filter(Boolean);
    if (reported.includes(name)) return;
    storage.setItem(key, [...reported, name].join(','));
  } catch {
    return;
  }
  void gateway.recordStep({ ...step, surface });
}
