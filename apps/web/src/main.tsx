import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';

import { AppProviders } from '@/app/app-providers';
import { ConfigurationFailure } from '@/app/configuration-failure';
import { loadRuntimeConfig } from '@/shared/config/runtime-config';
import { i18n, initializeI18n, resolveSystemLanguage } from '@/shared/i18n/i18n';
import { resources } from '@/shared/i18n/resources';

async function bootstrap(): Promise<void> {
  await initializeI18n();
  const rootElement = document.querySelector('#root');
  if (!(rootElement instanceof HTMLElement)) {
    throw new Error('Root element is missing.');
  }
  const config = loadRuntimeConfig();
  createRoot(rootElement).render(
    <StrictMode>
      <I18nextProvider i18n={i18n}>
        {config.ok ? (
          <AppProviders config={config.value} />
        ) : (
          <ConfigurationFailure issues={config.error.issues} />
        )}
      </I18nextProvider>
    </StrictMode>,
  );
}

void bootstrap().catch(() => {
  const language = resolveSystemLanguage(window.navigator.languages);
  const messages = resources[language].translation;
  const rootElement = document.querySelector('#root');
  if (rootElement instanceof HTMLElement) {
    const main = document.createElement('main');
    const title = document.createElement('h1');
    const detail = document.createElement('p');
    main.style.padding = '2rem';
    title.textContent = messages['bootstrap.failure.title'];
    detail.textContent = messages['bootstrap.failure.detail'];
    main.append(title, detail);
    rootElement.replaceChildren(main);
  }
  console.error(messages['bootstrap.failure.log']);
});
