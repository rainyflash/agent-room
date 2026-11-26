import { CopyBlock } from '@agent-room/ui-system';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { PrivateRoom, PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { RepositoryClaim } from '@/features/private-rooms/domain/project-rooms';
import type { Result } from '@/shared/result';

/** 在等的认领隔多久再问一次：后台每分钟核对一轮，认上了这里最多晚 20 秒知道。 */
export const PROJECT_CLAIM_POLL_MS = 20_000;

/** 给 GitHub 项目建房要用到的几样；没有认领这项服务时（测试、夹具）不给这个选项。 */
export type ProjectRoomSource = {
  /** 写给 Agent 的命令行怎么起头：桌面端是实际安装路径，网页端是 `agent-room`。 */
  readonly invocation: string;
  /** 网页端不知道 Agent 那台电脑把命令行装在哪：话的最后说一句去哪找。 */
  readonly locate: boolean;
  /** 这个账户在等的认领。 */
  readonly claims: () => Promise<Result<readonly RepositoryClaim[], PrivateRoomFailure>>;
  /** 认上以后建好的那一间：我建的、名字就是这个仓库的那间房；没有是 `null`。 */
  readonly findRoom: (repository: string) => Promise<PrivateRoom | null>;
};

/**
 * “新建房间”里给一个 GitHub 项目建房（`specs/redesign/project-front-door.md`）：网页不知道是哪个仓库，
 * 也碰不到仓库里的文件，所以给一段话，让在项目里干活的 Agent 认领、写 `AGENTS.md`，人推上去以后由服务器
 * 认上、建房。下面列这个账户在等的认领；有一个从列表里没了、又多了一间同名的房，就是认上了。
 */
export function NewProjectRoom({
  source,
  ready,
  onReady,
}: {
  readonly source: ProjectRoomSource;
  /** 认上以后建好的那一间；有了以后复制退成灰的，明黄的是对话框底下的“进房间”。 */
  readonly ready: PrivateRoom | null;
  readonly onReady: (room: PrivateRoom) => void;
}) {
  const { t } = useTranslation();
  const waiting = useWaitingClaims(source, onReady);
  const message = [
    t('rooms.create.projectMessage', {
      command: `${source.invocation} project`,
      wait: `${source.invocation} project --wait`,
    }),
    ...(source.locate ? ['', t('agentInvite.cli.locate')] : []),
  ].join('\n');

  return (
    <section className="new-room__project">
      <p>{t('rooms.create.projectHow')}</p>
      <CopyBlock
        copiedLabel={t('rooms.create.projectCopied')}
        copyLabel={t('rooms.create.projectCopy')}
        failedLabel={t('rooms.create.projectCopyFailed')}
        text={message}
        textLabel={t('rooms.create.projectMessageLabel')}
        tone={ready === null ? 'primary' : 'ghost'}
      />
      <ul className="new-room__project-facts">
        <li>{t('rooms.create.projectComputer')}</li>
        <li>{t('rooms.create.projectGithub')}</li>
        <li>{t('rooms.create.projectPublic')}</li>
        <li>{t('rooms.create.projectLimit')}</li>
      </ul>
      {waiting.length === 0 ? null : (
        <section className="new-room__project-waiting">
          <h3>{t('rooms.create.projectWaiting')}</h3>
          <ul>
            {waiting.map((claim) => (
              <li key={claim.repository}>
                <strong>{claim.repository}</strong>
                {t('rooms.create.projectClaim', {
                  result:
                    claim.lastCheck === null
                      ? t('rooms.create.projectNotChecked')
                      : t(`projectRooms.result.${claim.lastCheck.result}`),
                })}
              </li>
            ))}
          </ul>
        </section>
      )}
      {ready === null ? null : (
        <p className="new-room__project-ready" role="status">
          {t('rooms.create.projectReady', { repository: ready.name })}
        </p>
      )}
    </section>
  );
}

/**
 * 对话框开着时隔一会儿问一次在等的认领（只看要新建一间房的，绑已有房间的不在这里）。认上了的认领会被删掉，
 * 所以一个仓库从列表里没了，就去房间列表里找它建好的那一间；过期、撤掉的找不到，就只是从列表里消失。
 */
function useWaitingClaims(
  source: ProjectRoomSource,
  onReady: (room: PrivateRoom) => void,
): readonly RepositoryClaim[] {
  const [waiting, setWaiting] = useState<readonly RepositoryClaim[]>([]);
  const latest = useRef({ onReady, source });
  useEffect(() => {
    latest.current = { onReady, source };
  });
  useEffect(() => {
    const run = { looking: false, stopped: false };
    // await 之后要重新读：经函数读，类型检查才不会把它当成一直是 false。
    const stopped = () => run.stopped;
    let seen: ReadonlySet<string> = new Set();
    const look = async (): Promise<void> => {
      if (run.looking) return;
      run.looking = true;
      try {
        const result = await latest.current.source.claims();
        if (stopped() || !result.ok) return;
        const claims = result.value.filter((claim) => claim.catalogId === null);
        const now = new Set(claims.map((claim) => claim.repository));
        const gone = [...seen].filter((repository) => !now.has(repository));
        seen = now;
        setWaiting(claims);
        for (const repository of gone) {
          const room = await latest.current.source.findRoom(repository);
          if (stopped()) return;
          if (room !== null) {
            latest.current.onReady(room);
            return;
          }
        }
      } catch {
        // 问不到就等下一次。
      } finally {
        run.looking = false;
      }
    };
    void look();
    const timer = window.setInterval(() => void look(), PROJECT_CLAIM_POLL_MS);
    return () => {
      run.stopped = true;
      window.clearInterval(timer);
    };
  }, []);
  return waiting;
}
