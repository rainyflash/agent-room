import { createContext, useContext, type PropsWithChildren } from 'react';

const OverlayContainer = createContext<HTMLElement | null>(null);

export function OverlayContainerProvider({
  children,
  container,
}: PropsWithChildren<{ readonly container: HTMLElement | null }>) {
  return <OverlayContainer.Provider value={container}>{children}</OverlayContainer.Provider>;
}

export function useOverlayContainer(): HTMLElement {
  return useContext(OverlayContainer) ?? document.body;
}
