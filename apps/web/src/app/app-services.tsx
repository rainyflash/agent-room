import { createContext, useContext, type PropsWithChildren } from 'react';

import type { DirectSessionCoordinator } from '@/features/direct-sessions/application/direct-session-coordinator';
import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import type { DirectSessionGateway } from '@/features/direct-sessions/domain/direct-session';
import type { AutomationGrantGateway } from '@/features/automation/domain/automation-grant';
import type { HandoffGateway } from '@/features/handoffs/domain/handoff';
import type { ReadinessGateway } from '@/features/health/domain/readiness';
import type { LobbyGateway } from '@/features/lobby/domain/lobby';
import type { PublicLobbyEntryCoordinator } from '@/features/lobby-entry/application/public-lobby-entry-coordinator';
import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { MessageGateway } from '@/features/messages/domain/message';
import type { MachineTranslationGateway } from '@/features/messages/domain/machine-translation';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type { ModerationGateway } from '@/features/moderation/domain/moderation';
import type { OnboardingCoordinator } from '@/features/onboarding/application/onboarding-coordinator';
import type {
  PrivateRoomGateway,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type { MatrixSecurityGateway } from '@/features/security/domain/matrix-security';
import type { ControlPlaneGateway, SessionDependencies } from '@/features/session/domain/session';
import type { FrontendTelemetryGateway } from '@/features/telemetry/domain/frontend-metric';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

export type AppServices = {
  readonly accessManagement: AccessManagementGateway;
  readonly automation: AutomationGrantGateway;
  readonly config: RuntimeConfig;
  readonly content: ContentGateway;
  readonly contentVerifier: ContentVerifier;
  readonly controlPlane: ControlPlaneGateway & ReadinessGateway;
  readonly directSessionCoordinator: DirectSessionCoordinator;
  readonly directSessions: DirectSessionGateway;
  readonly desktop: DesktopRuntimeGateway;
  readonly handoffs: HandoffGateway;
  readonly lobby: LobbyGateway;
  readonly lobbyEntry: PublicLobbyEntryCoordinator;
  readonly messages: MessageGateway;
  readonly messageTranslation: MachineTranslationGateway;
  readonly messagePublisher: MessagePublisher;
  readonly moderation: ModerationGateway;
  readonly onboarding: OnboardingCoordinator;
  readonly privateRoomMatrix: PrivateRoomMatrixGateway;
  readonly privateRooms: PrivateRoomGateway;
  readonly security: MatrixSecurityGateway;
  readonly session: SessionDependencies;
  readonly telemetry: FrontendTelemetryGateway;
};

export type RuntimeMode = 'desktop' | 'web';

export type DesktopAppServices = {
  readonly config: RuntimeConfig;
  readonly desktop: DesktopRuntimeGateway;
  readonly runtimeMode: 'desktop';
};

export type WebAppServices = AppServices & {
  readonly runtimeMode: 'web';
};

export type RuntimeServices = DesktopAppServices | WebAppServices;

const AppServicesContext = createContext<RuntimeServices | null>(null);

export type AppServicesProviderProps = PropsWithChildren<{
  readonly services: RuntimeServices;
}>;

export function AppServicesProvider({ children, services }: AppServicesProviderProps) {
  return <AppServicesContext.Provider value={services}>{children}</AppServicesContext.Provider>;
}

export function useAppServices(): AppServices {
  const services = useContext(AppServicesContext);
  if (services === null) {
    throw new Error('AppServicesProvider is missing.');
  }
  if (services.runtimeMode !== 'web') {
    throw new Error('Web services are unavailable in the desktop runtime.');
  }
  return services;
}

export function useRuntimeServices(): RuntimeServices {
  const services = useContext(AppServicesContext);
  if (services === null) {
    throw new Error('AppServicesProvider is missing.');
  }
  return services;
}
