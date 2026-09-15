import { describe, expect, it, vi } from 'vitest';
import { ControlPlaneAgentRosterPolicy } from './control-plane-agent-roster-policy';

describe('room archive policy', () => {
  it('saves a room-wide rule using the current authenticated session', async () => {
    const request = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(JSON.stringify({ schemaVersion: 1, archiveAfterDays: 30 })));
    const client = new ControlPlaneAgentRosterPolicy('https://control.test/api', request);
    expect(await client.update('room', { schemaVersion: 1, archiveAfterDays: 30 })).toEqual({
      ok: true,
      value: { schemaVersion: 1, archiveAfterDays: 30 },
    });
    expect(request).toHaveBeenCalledWith(
      new URL('https://control.test/api/rooms/room/agent-roster-policy'),
      expect.objectContaining({
        method: 'PUT',
        credentials: 'include',
        body: '{"archiveAfterDays":30}',
      }),
    );
  });
  it.each([403, 503])('does not report a %i response as saved', async (status) => {
    const request = vi.fn<typeof fetch>().mockResolvedValue(new Response('', { status }));
    expect(
      (
        await new ControlPlaneAgentRosterPolicy('https://control.test/api', request).update(
          'room',
          { schemaVersion: 1, archiveAfterDays: 7 },
        )
      ).ok,
    ).toBe(false);
  });
});
