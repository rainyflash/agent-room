import { createActor, waitFor } from 'xstate';
import { describe, expect, it, vi } from 'vitest';

import {
  createContentInspectionMachine,
  type ContentInspectionDependencies,
  type ContentInspectionRequest,
} from './content-inspection-machine';
import type {
  ContentGateway,
  ContentVerifier,
  DownloadedContent,
  VerifiedContent,
} from '@/features/messages/domain/content';
import { err, ok } from '@/shared/result';

const request: ContentInspectionRequest = {
  roomId: '!room:matrix.test',
  matrixEventId: '$event',
  messageId: '01990d9e-8400-7000-8000-000000000003',
  reference: {
    contentId: '01990d9e-8400-7000-8000-000000000006',
    digestSha256: 'ab'.repeat(32),
    mediaType: 'text/plain',
    sizeBytes: 5,
  },
};
const downloaded: DownloadedContent = {
  bytes: new TextEncoder().encode('hello'),
  contentDigest: 'sha-256=:q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s=:',
  contentLength: '5',
  mediaType: 'text/plain',
};
const verified: VerifiedContent = {
  bytes: downloaded.bytes,
  digestSha256: 'ab'.repeat(32),
  mediaType: 'text/plain',
  mode: 'text',
  text: 'hello',
};

describe('正文检视状态机', () => {
  it('停留在空闲态时不会申请票据或下载正文', () => {
    const runtime = dependencies();
    const actor = createActor(createContentInspectionMachine(runtime.value)).start();

    expect(actor.getSnapshot().matches('idle')).toBe(true);
    expect(runtime.issueReadTicket).not.toHaveBeenCalled();
    expect(runtime.download).not.toHaveBeenCalled();
    expect(runtime.verify).not.toHaveBeenCalled();
    actor.stop();
  });

  it('点击后严格按票据、下载、校验三个阶段进入就绪态', async () => {
    const runtime = dependencies();
    const actor = createActor(createContentInspectionMachine(runtime.value)).start();

    actor.send({ request, type: 'OPEN' });
    const ready = await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    expect(runtime.issueReadTicket).toHaveBeenCalledWith(request.reference.contentId);
    expect(runtime.download).toHaveBeenCalledWith(
      request.reference.contentId,
      'short-lived-ticket',
    );
    expect(runtime.verify).toHaveBeenCalledWith(downloaded, request.reference, request.roomId);
    expect(ready.context.content).toEqual(verified);
    expect(ready.context.downloaded).toBeNull();
    expect(ready.context.ticket).toBeNull();
    actor.stop();
  });

  it('失败后只在用户明确重试时重新申请票据', async () => {
    const runtime = dependencies({
      issueReadTicket: vi
        .fn<ContentGateway['issueReadTicket']>()
        .mockResolvedValueOnce(err({ code: 'content.ticket_rejected', retryable: true }))
        .mockResolvedValue(
          ok({ expiresAtUnixMs: 1_800_000_000_000, ticket: 'short-lived-ticket' }),
        ),
    });
    const actor = createActor(createContentInspectionMachine(runtime.value)).start();
    actor.send({ request, type: 'OPEN' });
    await waitFor(actor, (snapshot) => snapshot.matches('failed'));

    expect(runtime.issueReadTicket).toHaveBeenCalledOnce();
    expect(runtime.download).not.toHaveBeenCalled();

    actor.send({ type: 'RETRY' });
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    expect(runtime.issueReadTicket).toHaveBeenCalledTimes(2);
    expect(runtime.download).toHaveBeenCalledOnce();
    actor.stop();
  });

  it('关闭检视器会清除正文、票据和请求上下文', async () => {
    const runtime = dependencies();
    const actor = createActor(createContentInspectionMachine(runtime.value)).start();
    actor.send({ request, type: 'OPEN' });
    await waitFor(actor, (snapshot) => snapshot.matches('ready'));

    actor.send({ type: 'CLOSE' });

    expect(actor.getSnapshot().matches('idle')).toBe(true);
    expect(actor.getSnapshot().context).toEqual({
      content: null,
      downloaded: null,
      failure: null,
      request: null,
      ticket: null,
    });
    actor.stop();
  });
});

function dependencies(
  overrides: { readonly issueReadTicket?: ContentGateway['issueReadTicket'] } = {},
) {
  const issueReadTicket =
    overrides.issueReadTicket ??
    vi.fn<ContentGateway['issueReadTicket']>(() =>
      Promise.resolve(ok({ expiresAtUnixMs: 1_800_000_000_000, ticket: 'short-lived-ticket' })),
    );
  const download = vi.fn<ContentGateway['download']>(() => Promise.resolve(ok(downloaded)));
  const verify = vi.fn<ContentVerifier['verify']>(() => Promise.resolve(ok(verified)));
  const content: ContentGateway = { download, issueReadTicket };
  const verifier: ContentVerifier = { verify };
  const value: ContentInspectionDependencies = { content, verifier };
  return { download, issueReadTicket, value, verify };
}
