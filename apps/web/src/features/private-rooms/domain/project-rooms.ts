import { z } from 'zod';

import type { Result } from '@/shared/result';
import type { PrivateRoomFailure } from './private-room';

/**
 * 项目房间（`specs/redesign/project-rooms.md`）：一个 GitHub 上的公开仓库绑一间公开房间。认领的人在仓库
 * 默认分支的 `AGENTS.md` 或 `README.md` 里放一行带这间房分享码的链接，服务器读到了就算认上。
 *
 * 认领和绑定都是新接口（3P-2b），回答单独校验，不往公开设置、房间的回答里加字段：那两份网页上也是
 * `.strict()`，旧版桌面端多看到一个字段整页读不出来。绑定的细节（谁什么时候在哪个文件里核对上的）只有房主
 * 问得到，别人问回 403；房间绑着哪个仓库谁登录了都问得到（房间名旁边的勾），它本来就在 `/gh/` 上公开着。
 */

const unixMillisSchema = z.number().int().nonnegative();

/** 6 位大小写字母和数字，和公开房间的分享码是同一套。 */
const shareCodeSchema = z.string().regex(/^[A-Za-z0-9]{6}$/u);

/** owner：字母、数字、连字符，1 到 39 个，不以连字符开头（和控制面 `GithubRepository` 同一条规矩）。 */
const ownerPattern = /^[A-Za-z0-9][A-Za-z0-9-]{0,38}$/u;
/** repo：字母、数字、`.`、`_`、`-`，1 到 100 个，不能是 `.` 或 `..`。 */
const namePattern = /^[A-Za-z0-9._-]{1,100}$/u;

function validRepository(value: string): boolean {
  const parts = value.split('/');
  if (parts.length !== 2) return false;
  const [owner = '', name = ''] = parts;
  return ownerPattern.test(owner) && namePattern.test(name) && name !== '.' && name !== '..';
}

export const githubRepositorySchema = z.string().refine(validRepository);

/**
 * 输入框里写的仓库：`owner/repo`，也认从浏览器地址栏贴过来的 `https://github.com/owner/repo`（末尾的
 * `.git`、斜杠去掉）。不合规矩回 `null`，不发请求。
 */
export function parseRepositoryInput(text: string): string | null {
  let value = text.trim();
  value = value.replace(/^(?:https?:\/\/)?(?:www\.)?github\.com\//iu, '');
  value = value.replace(/\/+$/u, '').replace(/\.git$/iu, '');
  return validRepository(value) ? value : null;
}

/** 等着的认领上次核对的结果（控制面 `ClaimCheckResult`）。 */
export const repositoryClaimCheckResults = [
  'checking',
  'not_in_repository',
  'repository_renamed',
  'file_too_large',
  'github_unavailable',
  'repository_taken',
  'creation_failed',
] as const;

export type RepositoryClaimCheckResult = (typeof repositoryClaimCheckResults)[number];

/** 房主“再核对一次”的结果：找到了，或者和认领一样说没找到的原因。 */
export const repositoryBindingCheckResults = ['found', ...repositoryClaimCheckResults] as const;

export type RepositoryBindingCheckResult = (typeof repositoryBindingCheckResults)[number];

export const repositoryFiles = ['AGENTS.md', 'README.md'] as const;

export type RepositoryFile = (typeof repositoryFiles)[number];

const shareUrlSchema = z.url({ protocol: /^https?$/u });

export const repositoryClaimSchema = z
  .object({
    /** 认上以后认领就没了，是 `null`。 */
    claimId: z.uuid().nullable(),
    repository: githubRepositorySchema,
    shareCode: shareCodeSchema,
    /** 服务器照部署的站点拼好的分享链接、`AGENTS.md` 那一行、README 徽章那一行：网页不知道裸域在哪。 */
    shareUrl: shareUrlSchema,
    agentsLine: z.string().min(1),
    badgeMarkdown: z.string().min(1),
    status: z.enum(['pending', 'bound']),
    /** 要绑的那间房；认上以后是绑上的那间房。 */
    catalogId: z.uuid().nullable(),
    claimedAtUnixMs: unixMillisSchema.nullable(),
    lastCheck: z
      .object({ atUnixMs: unixMillisSchema, result: z.enum(repositoryClaimCheckResults) })
      .strict()
      .nullable(),
  })
  .strict();

export type RepositoryClaim = z.infer<typeof repositoryClaimSchema>;

/** `GET /project-rooms/claims`：自己在等的认领，最早的在前。 */
export const repositoryClaimListSchema = z
  .object({ claims: z.array(repositoryClaimSchema) })
  .strict();

/** `https://github.com/owner/repo`，只认 github.com 上的 https 地址。 */
const githubUrlSchema = z.url({ hostname: /^github\.com$/u, protocol: /^https$/u });

/** 谁都能看的那一份：房间绑着的仓库和它在 GitHub 上的地址。 */
export const publicRoomRepositorySchema = z
  .object({ repository: githubRepositorySchema, repositoryUrl: githubUrlSchema })
  .strict();

export type PublicRoomRepository = z.infer<typeof publicRoomRepositorySchema>;

/** `GET /rooms/{c}/repository`：没有这间房、没绑、不公开了都回 `null`。 */
export const publicRoomRepositoryResponseSchema = publicRoomRepositorySchema.nullable();

export const roomRepositoryBindingSchema = z
  .object({
    repository: githubRepositorySchema,
    repositoryUrl: githubUrlSchema,
    verifiedByAccountId: z.uuid(),
    verifiedAtUnixMs: unixMillisSchema,
    verifiedFile: z.enum(repositoryFiles),
    /** 绑着仓库的房间都公开着、有分享码；万一没有，这几样是 `null`。 */
    shareCode: shareCodeSchema.nullable(),
    shareUrl: shareUrlSchema.nullable(),
    agentsLine: z.string().min(1).nullable(),
    badgeMarkdown: z.string().min(1).nullable(),
  })
  .strict();

export type RoomRepositoryBinding = z.infer<typeof roomRepositoryBindingSchema>;

/** `GET /private-rooms/{c}/repository`：没绑回 `null`。 */
export const roomRepositoryBindingResponseSchema = roomRepositoryBindingSchema.nullable();

export const roomRepositoryCheckSchema = z
  .object({
    binding: roomRepositoryBindingSchema,
    lastCheck: z
      .object({ atUnixMs: unixMillisSchema, result: z.enum(repositoryBindingCheckResults) })
      .strict(),
  })
  .strict();

export type RoomRepositoryCheck = z.infer<typeof roomRepositoryCheckSchema>;

export type ClaimRepositoryInput = {
  readonly repository: string;
  /** 绑这间已有的公开房间；不给就是认上了再建一间。 */
  readonly catalogId?: string;
};

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

export type ProjectRoomGateway = {
  /**
   * Claims a repository. The same person claiming the same repository gets the same claim back; a
   * repository already bound to one of their rooms answers `bound` with that room.
   */
  claim(input: ClaimRepositoryInput): Answer<RepositoryClaim>;
  /** Fails with `project_room.not_found` once the claim was bound (it is deleted) or expired. */
  inspectClaim(claimId: string): Answer<RepositoryClaim>;
  /** Checks now; once a minute per claim, `project_room.check_too_soon` (429) with `retryAfterSeconds`. */
  checkClaim(claimId: string): Answer<RepositoryClaim>;
  cancelClaim(claimId: string): Answer<void>;
  /** The signed-in person's claims that are still waiting, oldest first. */
  claims(): Answer<readonly RepositoryClaim[]>;
  /**
   * Anyone signed in: the repository a public room is linked to, for the mark next to its name. `null`
   * when the room doesn't exist, isn't linked or isn't public.
   */
  roomRepository(catalogId: string): Answer<PublicRoomRepository | null>;
  /** Owner only. `null` when the room isn't bound. */
  binding(catalogId: string): Answer<RoomRepositoryBinding | null>;
  /**
   * Owner only. Fails with `private_room.public_limit_reached` when the owner already has another public
   * room that isn't linked to a repository.
   */
  unbind(catalogId: string): Answer<void>;
  /** Owner only; once a minute per room. A missing line is reported but the binding stays. */
  checkBinding(catalogId: string): Answer<RoomRepositoryCheck>;
};
