import { describe, expect, it } from 'vitest';

import {
  SHARE_CARD_QR,
  shareCardFileName,
  shareCardQrLayout,
  shareCardSite,
  wrapCardText,
} from './share-card';

/** 每个字 10 宽：好算。 */
const measure = (text: string) => Array.from(text).length * 10;

describe('邀请卡片上的字', () => {
  it('放得下就是一行', () => {
    expect(wrapCardText('Builders Exchange', 200, 2, measure)).toEqual(['Builders Exchange']);
  });

  it('英文在空格处断，整个词挪到下一行', () => {
    expect(wrapCardText('Builders Exchange', 120, 2, measure)).toEqual(['Builders', 'Exchange']);
    expect(wrapCardText('Builders Exchange weekly', 120, 2, measure)).toEqual([
      'Builders',
      'Exchange we…',
    ]);
  });

  it('中文一个字一个字地断；超过行数的最后一行截短加“…”', () => {
    expect(wrapCardText('周三晚上的读书会', 40, 2, measure)).toEqual(['周三晚上', '的读书会']);
    expect(wrapCardText('周三晚上的读书会和周末的观影会', 40, 2, measure)).toEqual([
      '周三晚上',
      '的读书…',
    ]);
  });

  it('一个词比一行还宽：硬断，不丢字', () => {
    expect(wrapCardText('Supercalifragilistic', 80, 3, measure)).toEqual([
      'Supercal',
      'ifragili',
      'stic',
    ]);
  });

  it('多余的空白压成一个空格，空的不出行', () => {
    expect(wrapCardText('  a   b  ', 100, 2, measure)).toEqual(['a b']);
    expect(wrapCardText('', 100, 2, measure)).toEqual([]);
  });

  it('文件名：系统不让用的字换掉，空的有个兜底', () => {
    expect(shareCardFileName('读书会 / 周三: 晚上?')).toBe('agent-room-读书会 周三 晚上.png');
    expect(shareCardFileName('///')).toBe('agent-room.png');
    expect(shareCardFileName('a'.repeat(80))).toBe(`agent-room-${'a'.repeat(40)}.png`);
  });

  it('二维码一格占整数个像素，整块在白底里、四边留的白不少于定的', () => {
    for (const modules of [21, 25, 29, 33, 37, 41, 45, 49, 57, 77]) {
      const { cell, x, y } = shareCardQrLayout(modules);
      expect([cell, x, y].every(Number.isInteger)).toBe(true);
      expect(x).toBe(y - SHARE_CARD_QR.y + SHARE_CARD_QR.x);
      expect(x - SHARE_CARD_QR.x).toBeGreaterThanOrEqual(SHARE_CARD_QR.padding);
      expect(x + cell * modules).toBeLessThanOrEqual(
        SHARE_CARD_QR.x + SHARE_CARD_QR.size - SHARE_CARD_QR.padding,
      );
    }
    expect(shareCardQrLayout(37)).toEqual({ cell: 10, x: 170, y: 590 });
  });

  it('右下角只写主机名', () => {
    expect(shareCardSite('https://app.agentroom.chat/join#secret')).toBe('app.agentroom.chat');
    expect(shareCardSite('not a url')).toBe('');
  });
});
