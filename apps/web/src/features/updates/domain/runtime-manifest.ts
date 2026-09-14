import { z } from 'zod';

declare const __AGENT_ROOM_WRITE_CONTRACT__: string;
export const currentWriteContract = __AGENT_ROOM_WRITE_CONTRACT__;
const manifestSchema = z.object({
  schema: z.literal(1),
  writeContract: z.string().regex(/^[a-f0-9]{64}$/u),
  windowsDownloadUrl: z
    .url()
    .refine((url) => url.startsWith('https://'))
    .nullable(),
});
export type RuntimeManifest = z.infer<typeof manifestSchema>;

export async function readRuntimeManifest(signal?: AbortSignal): Promise<RuntimeManifest> {
  const timeout = AbortSignal.timeout(10_000);
  const response = await fetch('/runtime-manifest.json', {
    cache: 'no-store',
    signal: signal === undefined ? timeout : AbortSignal.any([signal, timeout]),
  });
  if (!response.ok) throw new Error('update.manifest_unavailable');
  return manifestSchema.parse(await response.json());
}
