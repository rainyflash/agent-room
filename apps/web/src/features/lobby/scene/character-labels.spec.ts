import { describe, expect, it } from 'vitest';
import { characterLabel, visibleCharacterLabels } from './character-labels';
import type { SceneCharacter } from './scene-character';

const person = (id: string, x = 0): SceneCharacter => ({
  characterId: id,
  matrixUserId: `@${id}:example.org`,
  displayName: '张小明的设计助手',
  kind: 'agent',
  isSelf: false,
  status: 'idle',
  radius: 27,
  x,
  y: 0,
});
describe('room character labels', () => {
  it('keeps the selected name readable without overlapping other names', () => {
    const people = [person('a'), person('b', 40), person('c', 800)];
    expect([...visibleCharacterLabels(people, 'b', true)]).toEqual(['b', 'c']);
    expect([...visibleCharacterLabels(people.toReversed(), 'b', true)]).toEqual(['b', 'c']);
  });
  it('shows only focused or human names at room overview scale', () => {
    expect([...visibleCharacterLabels([person('a'), person('b', 1000)], 'b', false)]).toEqual([
      'b',
    ]);
  });
  it('shortens long names without breaking a combined emoji', () => {
    const emoji = '👩‍💻';
    expect(characterLabel(emoji.repeat(21))).toBe(`${emoji.repeat(19)}…`);
  });
});
