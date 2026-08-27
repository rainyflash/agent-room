import { z } from 'zod';

import { err, ok, type Result } from '@/shared/result';

const originSchema = z
  .url()
  .transform((value) => new URL(value))
  .refine(
    (value) => value.pathname === '/' && value.search.length === 0 && value.hash.length === 0,
    'must be an origin without a path, query, or fragment',
  )
  .transform((value) => value.origin);

const optionalDownloadUrlSchema = z.preprocess(
  (value) => (typeof value === 'string' && value.trim() === '' ? null : (value ?? null)),
  z.url().nullable(),
);

const runtimeConfigSchema = z.object({
  controlPlaneUrl: originSchema,
  matrixHomeserverUrl: originSchema,
  windowsDownloadUrl: optionalDownloadUrlSchema,
});

export type RuntimeConfig = z.output<typeof runtimeConfigSchema>;

export type RuntimeConfigFailure = {
  readonly code: 'runtime.invalid_configuration';
  readonly issues: readonly string[];
};

export type RuntimeEnvironment = {
  readonly [key: string]: unknown;
  readonly VITE_AGENT_ROOM_CONTROL_PLANE_URL?: unknown;
  readonly VITE_AGENT_ROOM_MATRIX_HOMESERVER_URL?: unknown;
  readonly VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL?: unknown;
};

export function loadRuntimeConfig(
  environment: RuntimeEnvironment = import.meta.env,
): Result<RuntimeConfig, RuntimeConfigFailure> {
  const parsed = runtimeConfigSchema.safeParse({
    controlPlaneUrl:
      environment.VITE_AGENT_ROOM_CONTROL_PLANE_URL ?? 'https://api.agent-room.localhost:18443',
    matrixHomeserverUrl:
      environment.VITE_AGENT_ROOM_MATRIX_HOMESERVER_URL ??
      'https://matrix.agent-room.localhost:18443',
    windowsDownloadUrl: environment.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL,
  });

  if (parsed.success) {
    return ok(parsed.data);
  }

  return err({
    code: 'runtime.invalid_configuration',
    issues: parsed.error.issues.map((issue) => `${issue.path.join('.')}: ${issue.message}`),
  });
}
