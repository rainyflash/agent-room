import { BadgeCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import './repository-mark.css';

/** 房间绑着的 GitHub 仓库：`owner/repo` 和它在 GitHub 上的地址。 */
export type RoomRepositoryMark = {
  readonly repository: string;
  readonly url: string;
};

/** 房间名，绑着仓库时后面跟着勾；顶栏和场景版面的房间名都用它。 */
export function RoomTitle({
  name,
  repository,
}: {
  readonly name: string;
  readonly repository: RoomRepositoryMark | null;
}) {
  return repository === null ? (
    <h1>{name}</h1>
  ) : (
    <h1 className="room-name--verified">
      <span>{name}</span>
      <RepositoryMark {...repository} />
    </h1>
  );
}

/**
 * 房间名旁边的勾（`specs/redesign/project-rooms.md`“网页”）：绑着 GitHub 仓库的房间才有。指上去和读屏读的都是
 * “已验证的 GitHub 仓库 owner/repo”，点了在新标签页打开 GitHub。
 */
export function RepositoryMark({ repository, url }: RoomRepositoryMark) {
  const { t } = useTranslation();
  const label = t('projectRooms.verified', { repository });
  return (
    <a
      aria-label={label}
      className="room-repository-mark"
      href={url}
      rel="noopener noreferrer"
      target="_blank"
      title={label}
    >
      <BadgeCheck aria-hidden="true" />
    </a>
  );
}
