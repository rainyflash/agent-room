import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type Event, type UnlistenFn } from '@tauri-apps/api/event';
import { z } from 'zod';

import {
  bridgeRuntimeSchema,
  agentHostApplyReceiptSchema,
  agentHostDetectionSchema,
  agentHostPlanSchema,
  desktopAgentTargetSchema,
  desktopDeepLinkSchema,
  releaseUpdateCheckSchema,
  desktopRuntimeSnapshotSchema,
  type BridgeRuntime,
  type AgentHostDetection,
  type AgentHostKind,
  type AgentHostPlan,
  type DesktopRuntimeEventHandlers,
  type DesktopAgentTarget,
  type DesktopRuntimeFailure,
  type DesktopRuntimeGateway,
  type DesktopRuntimeSnapshot,
  type ReleaseUpdateChannel,
  type ReleaseUpdateCheck,
} from '@/features/desktop/domain/desktop-runtime';
import { err, ok, type Result } from '@/shared/result';

const commandFailureSchema = z
  .object({
    code: z
      .string()
      .min(1)
      .max(128)
      .regex(/^[a-z0-9._]+$/u),
    retryable: z.boolean(),
  })
  .strict();

const desktopCommands = {
  applyHost: 'desktop_apply_agent_host',
  authorization: 'desktop_open_authorization',
  autostart: 'desktop_set_autostart',
  bootstrapDefaultAgent: 'desktop_bootstrap_default_agent',
  checkUpdate: 'desktop_check_update',
  configureAgentRuntime: 'desktop_configure_agent_runtime',
  detectHosts: 'desktop_detect_agent_hosts',
  installUpdate: 'desktop_install_update',
  planHost: 'desktop_plan_agent_host',
  retry: 'desktop_retry_bridge',
  snapshot: 'desktop_runtime_snapshot',
} as const;

type DesktopCommand = (typeof desktopCommands)[keyof typeof desktopCommands];

export type TauriDesktopTransport = {
  readonly available: () => boolean;
  readonly invoke: (
    command: DesktopCommand,
    arguments_: Record<string, unknown>,
  ) => Promise<unknown>;
  readonly listen: (
    eventName: 'desktop://deep-link' | 'desktop://runtime-changed',
    listener: (payload: unknown) => void,
  ) => Promise<() => void>;
};

const nativeTransport: TauriDesktopTransport = {
  available: isTauri,
  invoke: (command, arguments_) => invoke<unknown>(command, arguments_),
  listen: async (eventName, listener) => {
    const unlisten: UnlistenFn = await listen<unknown>(eventName, (event: Event<unknown>) => {
      listener(event.payload);
    });
    return unlisten;
  },
};

export class TauriDesktopRuntimeGateway implements DesktopRuntimeGateway {
  constructor(private readonly transport: TauriDesktopTransport = nativeTransport) {}

  isAvailable(): boolean {
    return this.transport.available();
  }

  async snapshot(): Promise<Result<DesktopRuntimeSnapshot, DesktopRuntimeFailure>> {
    return this.invokeValidated(desktopCommands.snapshot, {}, desktopRuntimeSnapshotSchema);
  }

  async retryBridge(): Promise<Result<BridgeRuntime, DesktopRuntimeFailure>> {
    return this.invokeValidated(desktopCommands.retry, {}, bridgeRuntimeSchema);
  }

  async setAutostart(enabled: boolean): Promise<Result<boolean, DesktopRuntimeFailure>> {
    return this.invokeValidated(desktopCommands.autostart, { enabled }, z.boolean());
  }

  async openAuthorization(promptId: string): Promise<Result<void, DesktopRuntimeFailure>> {
    return this.invokeValidated(
      desktopCommands.authorization,
      { promptId },
      z
        .undefined()
        .or(z.null())
        .transform(() => undefined),
    );
  }

  async checkUpdate(
    channel: ReleaseUpdateChannel,
  ): Promise<Result<ReleaseUpdateCheck, DesktopRuntimeFailure>> {
    return this.invokeValidated(desktopCommands.checkUpdate, { channel }, releaseUpdateCheckSchema);
  }

  async installUpdate(
    channel: ReleaseUpdateChannel,
    expectedSequence: number,
  ): Promise<Result<void, DesktopRuntimeFailure>> {
    return this.invokeValidated(
      desktopCommands.installUpdate,
      { channel, expectedSequence },
      z
        .undefined()
        .or(z.null())
        .transform(() => undefined),
    );
  }

  async bootstrapDefaultAgent(
    preferredLanguage: string | null,
  ): Promise<Result<DesktopAgentTarget, DesktopRuntimeFailure>> {
    return this.invokeValidated(
      desktopCommands.bootstrapDefaultAgent,
      { preferredLanguage },
      desktopAgentTargetSchema,
    );
  }

  async configureAgentRuntime(
    target: DesktopAgentTarget,
  ): Promise<Result<DesktopAgentTarget, DesktopRuntimeFailure>> {
    return this.invokeValidated(
      desktopCommands.configureAgentRuntime,
      {
        agentId: target.agentId,
        lobbyLanguage: target.lobbyLanguage,
        publicLobbyCatalogId: target.publicLobbyCatalogId,
      },
      desktopAgentTargetSchema,
    );
  }

  async detectHosts(): Promise<Result<readonly AgentHostDetection[], DesktopRuntimeFailure>> {
    return this.invokeValidated(desktopCommands.detectHosts, {}, z.array(agentHostDetectionSchema));
  }

  async planHost(host: AgentHostKind): Promise<Result<AgentHostPlan, DesktopRuntimeFailure>> {
    return this.invokeValidated(desktopCommands.planHost, { host }, agentHostPlanSchema);
  }

  async applyHost(
    host: AgentHostKind,
    expectedOriginalDigest: string,
  ): Promise<Result<void, DesktopRuntimeFailure>> {
    const result = await this.invokeValidated(
      desktopCommands.applyHost,
      { expectedOriginalDigest, host },
      agentHostApplyReceiptSchema,
    );
    return result.ok ? ok(undefined) : result;
  }

  async subscribe(
    handlers: DesktopRuntimeEventHandlers,
  ): Promise<Result<() => void, DesktopRuntimeFailure>> {
    if (!this.transport.available()) {
      return err({ code: 'desktop.runtime.unavailable', retryable: false });
    }
    try {
      const removeRuntimeListener = await this.transport.listen(
        'desktop://runtime-changed',
        (payload) => {
          const parsed = bridgeRuntimeSchema.safeParse(payload);
          if (parsed.success) {
            handlers.onRuntimeChanged(parsed.data);
            return;
          }
          handlers.onFailure({ code: 'desktop.event.invalid_runtime', retryable: true });
        },
      );
      try {
        const removeDeepLinkListener = await this.transport.listen(
          'desktop://deep-link',
          (payload) => {
            const parsed = desktopDeepLinkSchema.safeParse(payload);
            if (parsed.success) {
              handlers.onDeepLink(parsed.data);
              return;
            }
            handlers.onFailure({ code: 'desktop.event.invalid_deep_link', retryable: false });
          },
        );
        return ok(() => {
          removeDeepLinkListener();
          removeRuntimeListener();
        });
      } catch (error: unknown) {
        removeRuntimeListener();
        return err(normalizeCommandFailure(error, 'desktop.event.subscribe_failed'));
      }
    } catch (error: unknown) {
      return err(normalizeCommandFailure(error, 'desktop.event.subscribe_failed'));
    }
  }

  private async invokeValidated<TValue>(
    command: DesktopCommand,
    arguments_: Record<string, unknown>,
    schema: z.ZodType<TValue>,
  ): Promise<Result<TValue, DesktopRuntimeFailure>> {
    if (!this.transport.available()) {
      return err({ code: 'desktop.runtime.unavailable', retryable: false });
    }
    try {
      const raw = await this.transport.invoke(command, arguments_);
      const parsed = schema.safeParse(raw);
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'desktop.command.invalid_response', retryable: true });
    } catch (error: unknown) {
      return err(normalizeCommandFailure(error, 'desktop.command.failed'));
    }
  }
}

function normalizeCommandFailure(error: unknown, fallbackCode: string): DesktopRuntimeFailure {
  const parsed = commandFailureSchema.safeParse(error);
  return parsed.success ? parsed.data : { code: fallbackCode, retryable: true };
}
