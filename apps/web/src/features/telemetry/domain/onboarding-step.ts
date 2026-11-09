/** 接入 Agent 对话框里只有前端知道的几步（specs/onboarding-funnel/design.md 第 2 节）。 */
export type OnboardingMethod = 'cli' | 'mcp' | 'network';

export type OnboardingStep =
  | { readonly step: 'desktop_download' | 'invite_opened' }
  | { readonly step: 'invite_arrived' | 'invite_copied'; readonly method: OnboardingMethod };

/** 只有步骤、接入方式和是网页还是桌面端，不带账户、房间和地址。 */
export type OnboardingStepSample = OnboardingStep & {
  readonly surface: 'desktop' | 'web';
};

export type OnboardingStepGateway = {
  recordStep(sample: OnboardingStepSample): Promise<void>;
};
