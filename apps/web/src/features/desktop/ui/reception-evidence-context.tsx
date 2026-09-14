import { createContext, useContext, type PropsWithChildren } from 'react';
import type { ReceiverView } from '../domain/reception';
import type { DesktopRuntimeGateway } from '../domain/desktop-runtime';
import { useReceptionQueries } from './use-reception-queries';

const empty: readonly ReceiverView[] = [];
const ReceptionEvidenceContext = createContext(empty);

export function ReceptionEvidenceProvider({
  children,
  gateway,
  principalId,
}: PropsWithChildren<{
  readonly gateway: DesktopRuntimeGateway;
  readonly principalId: string | null;
}>) {
  const { available, receivers } = useReceptionQueries(gateway, principalId);
  const views =
    available && receivers.data?.ok && !receivers.isError
      ? receivers.data.value.filter(
          (view) => view.state.binding.policy.allowedPrincipalId === principalId,
        )
      : empty;
  return (
    <ReceptionEvidenceContext.Provider value={views}>{children}</ReceptionEvidenceContext.Provider>
  );
}

export function useReceptionEvidence() {
  return useContext(ReceptionEvidenceContext);
}
