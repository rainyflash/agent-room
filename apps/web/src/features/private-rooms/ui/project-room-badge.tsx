import { useId } from 'react';
import { useTranslation } from 'react-i18next';

import { useRoomRepository } from '@/features/private-rooms/data/project-room-queries';
import type { ProjectRoomGateway } from '@/features/private-rooms/domain/project-rooms';

import { RepositoryLineCopy } from './project-room-repository';

export type RepositoryBadgeShareProps = {
  readonly catalogId: string;
  readonly gateway: ProjectRoomGateway;
};

/**
 * 房间“分享”对话框里的“README 徽章”一节（`specs/redesign/project-rooms.md`“README 徽章”，3P-8b-1）：徽章那一行
 * Markdown 和复制按钮。那一行是服务器照部署的站点拼好的，在只有房主问得到的绑定那一份里（和设置里“GitHub
 * 仓库”一组同一个查询、同一份缓存），网页不自己拼。外面那一层只在你是房主、房间绑着仓库时放它进来；没绑、
 * 还没问到、问不到、没有那一行时整节不出来，设置里那一组照旧什么都看得到。
 */
export function RepositoryBadgeShare({ catalogId, gateway }: RepositoryBadgeShareProps) {
  const { t } = useTranslation();
  const headingId = useId();
  const binding = useRoomRepository(gateway, catalogId);
  const badge = binding.data?.ok === true ? (binding.data.value?.badgeMarkdown ?? null) : null;
  if (badge === null) return null;
  const label = t('projectRooms.badge.label');
  return (
    <section aria-labelledby={headingId} className="repository-badge-share">
      <h3 id={headingId}>{label}</h3>
      <p className="repository-badge-share__note">{t('projectRooms.badge.shareDetail')}</p>
      <RepositoryLineCopy copyLabel={t('projectRooms.badge.copy')} text={badge} textLabel={label} />
    </section>
  );
}
