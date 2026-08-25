import { createContext, useContext, type PropsWithChildren } from 'react';

import type { HandoffGateway } from '@/features/handoffs/domain/handoff';
import type { LobbyGateway } from '@/features/lobby/domain/lobby';
import type { ContentGateway, ContentVerifier } from '@/features/messages/domain/content';
import type { MessageGateway } from '@/features/messages/domain/message';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type {
  PrivateRoomGateway,
  PrivateRoomMatrixGateway,
} from '@/features/private-rooms/domain/private-room';
import type { ControlPlaneClient } from '@/features/session/adapters/control-plane-client';
import type { RuntimeConfig } from '@/shared/config/runtime-config';

export type AppServices = {
  readonly config: RuntimeConfig;
  readonly content: ContentGateway;
  readonly contentVerifier: ContentVerifier;
  readonly controlPlane: ControlPlaneClient;
  readonly handoffs: HandoffGateway;
  readonly lobby: LobbyGateway;
  readonly messages: MessageGateway;
  readonly messagePublisher: MessagePublisher;
  readonly privateRoomMatrix: PrivateRoomMatrixGateway;
  readonly privateRooms: PrivateRoomGateway;
};

const AppServicesContext = createContext<AppServices | null>(null);

export type AppServicesProviderProps = PropsWithChildren<{
  readonly services: AppServices;
}>;

export function AppServicesProvider({ children, services }: AppServicesProviderProps) {
  return <AppServicesContext.Provider value={services}>{children}</AppServicesContext.Provider>;
}

export function useAppServices(): AppServices {
  const services = useContext(AppServicesContext);
  if (services === null) {
    throw new Error('AppServicesProvider is missing.');
  }
  return services;
}
