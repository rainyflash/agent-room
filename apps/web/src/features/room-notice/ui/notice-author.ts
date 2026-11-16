import type { TFunction } from 'i18next';

import type { LobbyMemberName } from '@/features/lobby/domain/lobby';

/** 改须知的人怎么称呼：自己写“你”，认得出的写昵称，认不出的写“房主或管理员”。 */
export function noticeAuthor(
  t: TFunction,
  updatedBy: string,
  memberNames: readonly LobbyMemberName[] | undefined,
  selfMatrixUserId: string | null,
): string {
  if (updatedBy === selfMatrixUserId) return t('roomNotice.you');
  return (
    memberNames?.find((member) => member.matrixUserId === updatedBy)?.displayName ??
    t('roomNotice.someone')
  );
}
