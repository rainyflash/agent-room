import type { AutomationGrantGateway } from '@/features/automation/domain/automation-grant';
import type { DesktopRuntimeGateway, HostSessionDiagnostics } from '../domain/desktop-runtime';
import { err, ok, type Result } from '@/shared/result';
import { enableReception } from './enable-reception';

/**
 * 接入对话框替人打开后台回复：这个会话已经接上了就不再动；不然读一次授权（有能用的就用），再走
 * “我的 Agent”里开启按钮的那三步（授权、绑定、启动）。对话框不常驻授权列表，所以在这里现读。
 */
export async function enableInviteBackground(
  input: {
    readonly session: HostSessionDiagnostics;
    readonly principalId: string;
    readonly grantId: string;
  },
  dependencies: {
    readonly automation: Pick<AutomationGrantGateway, 'list' | 'create'>;
    readonly runtime: Pick<
      DesktopRuntimeGateway,
      'listReceivers' | 'configureReceiver' | 'receiverAction'
    >;
  },
): Promise<Result<void, { readonly code: string }>> {
  const { session } = input;
  const { automation, runtime } = dependencies;
  if (runtime.listReceivers === undefined) {
    return err({ code: 'receiver.unavailable' });
  }
  const receivers = await runtime.listReceivers();
  if (
    receivers.ok &&
    session.sessionKey != null &&
    receivers.value.some((view) => view.state.binding.session.sessionKey === session.sessionKey)
  ) {
    return ok(undefined);
  }
  const grants = await automation.list();
  if (!grants.ok) return grants;
  return await enableReception(
    {
      session,
      principalId: input.principalId,
      grantId: input.grantId,
      grants: grants.value,
      now: Date.now(),
    },
    { automation, runtime },
  );
}
