import type { Result } from '@/shared/result';
import type { WorkspaceDocument, WorkspaceFailure } from './workspace-document';

export type WorkspaceScope = { readonly accountId: string; readonly writerId: string };
export type WorkspaceGateway = {
  readonly scope: () => WorkspaceScope | null;
  readonly read: (scope: WorkspaceScope) => Promise<Result<WorkspaceDocument, WorkspaceFailure>>;
  readonly write: (
    scope: WorkspaceScope,
    document: WorkspaceDocument,
  ) => Promise<Result<void, WorkspaceFailure>>;
  readonly subscribe: (listener: () => void) => () => void;
};
export type WorkspaceCache = {
  read(accountId: string): Result<WorkspaceDocument, WorkspaceFailure>;
  write(accountId: string, document: WorkspaceDocument): Result<void, WorkspaceFailure>;
};
