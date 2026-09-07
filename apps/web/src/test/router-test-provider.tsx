import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterContextProvider,
} from '@tanstack/react-router';
import { useState, type ReactNode } from 'react';

/** 给独立界面测试提供真实链接上下文；完整导航由应用浏览器测试验证。 */
export function RouterTestProvider({ children }: { readonly children: ReactNode }) {
  const [router] = useState(() => {
    const root = createRootRoute();
    return createRouter({
      history: createMemoryHistory({ initialEntries: ['/'] }),
      routeTree: root.addChildren(
        ['/', '/rooms', '/connect', '/workspace', '/settings/$section', '/onboarding'].map((path) =>
          createRoute({ getParentRoute: () => root, path }),
        ),
      ),
    });
  });
  return <RouterContextProvider router={router}>{children}</RouterContextProvider>;
}
