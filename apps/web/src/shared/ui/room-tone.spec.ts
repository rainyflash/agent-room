import { describe, expect, it } from 'vitest';

import { roomTone, roomTones } from './room-tone';

describe('房间小图标的颜色', () => {
  it('按房间号挑，同一间房总是同一种', () => {
    const id = '0198c000-0000-7000-8000-000000000001';
    expect(roomTone(id)).toBe(roomTone(id));
    expect(roomTones).toContain(roomTone(id));
  });

  it('四种都用得上', () => {
    const tones = new Set(
      Array.from({ length: 40 }, (_, index) =>
        roomTone(`0198c000-0000-7000-8000-${index.toString(16).padStart(12, '0')}`),
      ),
    );
    expect([...tones].sort()).toEqual([...roomTones].sort());
  });
});
