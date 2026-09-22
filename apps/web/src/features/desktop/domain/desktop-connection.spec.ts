import { describe, expect, it } from 'vitest';
import { haltReasonMessage } from './desktop-connection';

describe('halt reason copy', () => {
  it('把 Bridge 报的离线原因映射成能做的事，未知原因用通用说明', () => {
    expect(haltReasonMessage('bridge.secure_storage_corrupt')).toBe(
      'desktop.halted.reason.secureStorage',
    );
    expect(haltReasonMessage('bridge.matrix_crypto_identity_conflict')).toBe(
      'desktop.halted.reason.identityConflict',
    );
    expect(haltReasonMessage('bridge.matrix_room_not_found')).toBe(
      'desktop.halted.reason.roomMissing',
    );
    expect(haltReasonMessage('desktop.bridge.offline')).toBe('desktop.halted.description');
    expect(haltReasonMessage(null)).toBe('desktop.halted.description');
  });
});
