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
  it('reserves the drawn width, so neighbours with short Latin names both keep their names', () => {
    const named = (id: string, x: number): SceneCharacter => ({
      ...person(id, x),
      displayName: 'Build Agent 001',
    });
    // 旧的估算按每个字 18 px 预留，两个相隔 210 px 的英文名只能显示一个。
    expect([...visibleCharacterLabels([named('a', 0), named('b', 210)], null, true)]).toEqual([
      'a',
      'b',
    ]);
  });
  it('widens the reserved box for a status sticker longer than the name', () => {
    const short = (id: string, x: number): SceneCharacter => ({
      ...person(id, x),
      displayName: 'Ada',
    });
    const status = () => 'Waiting for input · Reads next run';
    expect([...visibleCharacterLabels([short('a', 0), short('b', 150)], null, true)]).toEqual([
      'a',
      'b',
    ]);
    expect([
      ...visibleCharacterLabels([short('a', 0), short('b', 150)], null, true, status),
    ]).toEqual(['a']);
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
