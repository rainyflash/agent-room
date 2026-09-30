import { Button, Field } from '@agent-room/ui-system';
import { Star } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { FavoriteUndo } from '../application/personal-workspace-store';
import { parseProjectTags } from '../domain/agent-organization';
import { usePersonalWorkspace } from './personal-workspace-provider';
import { WorkspaceSyncStatus } from './workspace-sync-status';
import './personal-workspace.css';

export function AgentOrganizationControls({ agentId }: { readonly agentId: string }) {
  const workspace = usePersonalWorkspace();
  return workspace?.snapshot.accountId ? (
    <OrganizationEditor key={`${workspace.snapshot.accountId}:${agentId}`} agentId={agentId} />
  ) : null;
}
function OrganizationEditor({ agentId }: { readonly agentId: string }) {
  const workspace = usePersonalWorkspace();
  const { t } = useTranslation();
  const [draft, setDraft] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<
    'saved' | 'unavailable' | 'tagsInvalid' | 'undoChanged' | null
  >(null);
  const [undo, setUndo] = useState<FavoriteUndo | null>(null);
  if (workspace?.snapshot.accountId == null) return null;
  const favorite = workspace.snapshot.index.favorites.has(agentId);
  const tags = workspace.snapshot.index.tags.get(agentId) ?? [];
  return (
    <section className="agent-organization" aria-label={t('personal.title')}>
      <div className="agent-organization__favorite">
        <Button
          aria-pressed={favorite}
          className="personal-favorite"
          icon={<Star aria-hidden="true" fill={favorite ? 'currentColor' : 'none'} />}
          onClick={() => {
            const result = workspace.toggleFavorite(agentId);
            setFeedback(result.ok ? 'saved' : 'unavailable');
            setUndo(result.ok ? result.value : null);
          }}
          size="compact"
          tone="quiet"
        >
          {t(favorite ? 'personal.unfavorite' : 'personal.favorite')}
        </Button>
        {undo ? (
          <Button
            onClick={() => {
              const result = workspace.undoFavorite(undo);
              setFeedback(
                result.ok
                  ? 'saved'
                  : result.error.code === 'workspace.undo_changed'
                    ? 'undoChanged'
                    : 'unavailable',
              );
              setUndo(null);
            }}
            size="compact"
            tone="quiet"
          >
            {t('personal.undo')}
          </Button>
        ) : null}
      </div>
      <form
        className="agent-organization__tags"
        onSubmit={(event) => {
          event.preventDefault();
          const value = parseProjectTags(draft ?? tags.join(', '));
          if (value === null) {
            setFeedback('tagsInvalid');
            return;
          }
          const result = workspace.change({ kind: 'tags', id: agentId, value: [...value] });
          setFeedback(result.ok ? 'saved' : 'unavailable');
          if (result.ok) setDraft(null);
        }}
      >
        <Field hint={t('personal.tagsHint')} label={t('personal.tags')}>
          <input
            value={draft ?? tags.join(', ')}
            maxLength={400}
            onChange={(event) => {
              setDraft(event.target.value);
            }}
          />
        </Field>
        <Button size="compact" tone="quiet" type="submit">
          {t('personal.saveTags')}
        </Button>
      </form>
      {feedback ? <p role="status">{t(`personal.${feedback}`)}</p> : null}
      <WorkspaceSyncStatus />
    </section>
  );
}
