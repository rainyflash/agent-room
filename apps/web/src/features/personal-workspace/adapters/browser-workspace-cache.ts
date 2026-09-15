import {
  emptyWorkspace,
  parseWorkspace,
  type WorkspaceDocument,
  type WorkspaceFailure,
} from '../domain/workspace-document';
import type { WorkspaceCache } from '../domain/workspace-gateway';
import { err, ok, type Result } from '@/shared/result';

export class BrowserWorkspaceCache implements WorkspaceCache {
  constructor(private readonly storage: Pick<Storage, 'getItem' | 'setItem'>) {}

  read(accountId: string): Result<WorkspaceDocument, WorkspaceFailure> {
    try {
      const serialized = this.storage.getItem(key(accountId));
      return serialized === null ? ok(emptyWorkspace) : parseWorkspace(JSON.parse(serialized));
    } catch {
      return err({ code: 'workspace.cache_read_failed', retryable: true });
    }
  }

  write(accountId: string, document: WorkspaceDocument): Result<void, WorkspaceFailure> {
    try {
      this.storage.setItem(key(accountId), JSON.stringify(document));
      return ok(undefined);
    } catch {
      return err({ code: 'workspace.cache_write_failed', retryable: true });
    }
  }
}

function key(accountId: string): string {
  return `agent-room.personal-workspace.v1:${encodeURIComponent(accountId)}`;
}
