import { createClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';
import { emptyWorkspace } from '../domain/workspace-document';
import { MatrixWorkspaceGateway, PERSONAL_WORKSPACE_EVENT_TYPE } from './matrix-workspace-gateway';

const scope = { accountId: '@new-user:matrix.example.test', writerId: 'DEVICE_A' };

function gatewayWithResponse(status: number, body: unknown) {
  const fetchFn = vi.fn<typeof fetch>().mockImplementation(() =>
    Promise.resolve(
      new Response(JSON.stringify(body), {
        status,
        headers: { 'Content-Type': 'application/json' },
      }),
    ),
  );
  const registry = new MatrixClientRegistry();
  registry.replace(
    createClient({
      baseUrl: 'https://matrix.example.test',
      accessToken: 'test-only-token',
      userId: scope.accountId,
      deviceId: scope.writerId,
      fetchFn,
    }),
  );
  return { gateway: new MatrixWorkspaceGateway(registry), fetchFn };
}

describe('个人工作区与真实 Matrix SDK 的数据边界', () => {
  it('SDK 将新账号的 M_NOT_FOUND 转为空工作区', async () => {
    const { gateway, fetchFn } = gatewayWithResponse(404, {
      errcode: 'M_NOT_FOUND',
      error: 'Account data not found.',
    });
    await expect(gateway.read(scope)).resolves.toEqual({ ok: true, value: emptyWorkspace });
    expect(fetchFn).toHaveBeenCalledOnce();
    const request = fetchFn.mock.calls[0]?.[0];
    const url =
      typeof request === 'string' ? request : request instanceof URL ? request.href : request?.url;
    expect(url).toContain(`/account_data/${PERSONAL_WORKSPACE_EVENT_TYPE}`);
  });

  it.each([
    [403, 'M_FORBIDDEN'],
    [404, 'M_UNRECOGNIZED'],
  ])('HTTP %s / %s 不会被当成空工作区', async (status, errcode) => {
    const { gateway } = gatewayWithResponse(status, { errcode, error: 'Expected failure.' });
    await expect(gateway.read(scope)).resolves.toEqual({
      ok: false,
      error: { code: 'workspace.read_failed', retryable: true },
    });
  });

  it('拒绝损坏的数据，而不是覆盖为空工作区', async () => {
    const { gateway } = gatewayWithResponse(200, { schema: 1, entries: 'invalid' });
    await expect(gateway.read(scope)).resolves.toEqual({
      ok: false,
      error: { code: 'workspace.invalid_document', retryable: false },
    });
  });
});
