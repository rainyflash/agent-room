import { Button } from '@agent-room/ui-system';
import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { FileSave } from './use-file-save';
import './file-save-note.css';

export type FileSaveNoteProps = {
  /** `useFileSave` 的 `last`；用的地方先看是不是眼前这一份内容的。 */
  readonly save: FileSave | null;
  readonly onShow: (() => void) | null;
};

/**
 * 桌面端存完文件以后的那一句：存到哪了、叫什么，能在文件夹里指出来；没存成也说。
 * 没有这一句，人点了“保存”窗口里什么都不变，以为没反应。
 */
export function FileSaveNote({ onShow, save }: FileSaveNoteProps) {
  const { t } = useTranslation();
  if (save === null || save.kind === 'saving') return null;
  if (save.kind === 'failed') {
    return (
      <div className="file-save-note">
        <p className="file-save-note__problem" role="alert">
          {t('fileSave.failed')}
        </p>
      </div>
    );
  }
  return (
    <div className="file-save-note">
      <p className="file-save-note__saved" role="status">
        {t('fileSave.saved', { fileName: save.fileName })}
      </p>
      {onShow === null ? null : (
        <Button
          icon={<FolderOpen aria-hidden="true" />}
          onClick={onShow}
          size="compact"
          tone="ghost"
        >
          {t('fileSave.show')}
        </Button>
      )}
      {save.showFailed ? (
        <p className="file-save-note__problem" role="alert">
          {t('fileSave.showFailed')}
        </p>
      ) : null}
    </div>
  );
}
