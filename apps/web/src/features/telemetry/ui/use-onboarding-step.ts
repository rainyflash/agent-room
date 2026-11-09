import { useCallback } from 'react';

import { useOptionalAppServices } from '@/app/app-services';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import { resolveFrontendSurface } from '@/features/telemetry/adapters/runtime-surface';
import { reportOnboardingStep } from '@/features/telemetry/application/onboarding-steps';
import type { OnboardingStep } from '@/features/telemetry/domain/onboarding-step';

function storageOrNull(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** 报接入 Agent 走到的一步。没登录、测试和夹具里什么都不做。 */
export function useOnboardingStep(): (step: OnboardingStep) => void {
  const gateway = useOptionalAppServices()?.onboardingSteps;
  const accountId = useOptionalSession()?.snapshot.context.principal?.principalId ?? null;
  return useCallback(
    (step: OnboardingStep) => {
      if (gateway === undefined || accountId === null) return;
      reportOnboardingStep(gateway, storageOrNull(), accountId, resolveFrontendSurface(), step);
    },
    [gateway, accountId],
  );
}
