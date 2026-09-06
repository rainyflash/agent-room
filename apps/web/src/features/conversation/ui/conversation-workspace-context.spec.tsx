// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { StrictMode, useState } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, expect, it } from 'vitest';
import { ConversationWorkspaceProvider } from './conversation-workspace-context';
import { ConversationPanel } from './conversation-panel';
import { initializeI18n, i18n } from '@/shared/i18n/i18n';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import { ok } from '@/shared/result';

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});
afterEach(cleanup);

const publisher: MessagePublisher = {
  resolveIdentity: () =>
    Promise.resolve(
      ok({
        kind: 'human',
        displayName: 'Operator',
        matrixUserId: '@operator:room.test',
        principalId: '01990d9e-8400-7000-8000-000000000003',
        source: 'matrix_human_session',
      }),
    ),
  publish: (request) =>
    Promise.resolve(
      ok({
        kind: 'published',
        submissionId: request.submissionId,
        matrixEventId: '$sent',
        reused: false,
      }),
    ),
  reconcile: (submissionId) =>
    Promise.resolve(ok({ kind: 'published', submissionId, matrixEventId: '$sent', reused: true })),
};

function Workspace({ account }: { readonly account: string }) {
  const [visible, setVisible] = useState(true);
  return (
    <StrictMode>
      <I18nextProvider i18n={i18n}>
        <ConversationWorkspaceProvider publisher={publisher} scope={account}>
          <button
            type="button"
            onClick={() => {
              setVisible((value) => !value);
            }}
          >
            Toggle conversation
          </button>
          {visible ? (
            <ConversationPanel
              publisher={publisher}
              roomId="!private:room.test"
              roomName="Private"
              state="ready"
              messages={[]}
            />
          ) : null}
        </ConversationWorkspaceProvider>
      </I18nextProvider>
    </StrictMode>
  );
}

it('真实组件卸载后可恢复草稿，切换账户后不显示上一个账户的草稿', async () => {
  const user = userEvent.setup();
  const view = render(<Workspace account="@owner-a:room.test" />);
  const input = () => screen.getByRole('textbox', { name: 'Message' });
  await waitFor(() => {
    expect(input()).toBeEnabled();
  });
  await user.type(input(), 'Private draft for account A');
  await user.click(screen.getByRole('button', { name: 'Toggle conversation' }));
  expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Toggle conversation' }));
  expect(input()).toHaveValue('Private draft for account A');
  view.rerender(<Workspace account="@owner-b:room.test" />);
  await waitFor(() => {
    expect(input()).toBeEnabled();
  });
  expect(input()).toHaveValue('');
  view.unmount();
  render(<Workspace account="@owner-a:room.test" />);
  await waitFor(() => {
    expect(input()).toBeEnabled();
  });
  expect(input()).toHaveValue('');
});
