import { describe, expect, it } from 'vitest';
import { suggestEmail } from '../../infra/identity/themes/agent-room/login/resources/js/email-suggestion.js';

describe('注册页的邮箱纠错提示', () => {
  it('常见邮箱打错了就给出改好的地址', () => {
    // 2026-11-08 生产上停在验证那一步的就有一个 gmail.comm。
    expect(suggestEmail('ada@gmail.comm')).toBe('ada@gmail.com');
    expect(suggestEmail('ada@gmial.com')).toBe('ada@gmail.com');
    expect(suggestEmail('ada@gmai.com')).toBe('ada@gmail.com');
    expect(suggestEmail('ada@gmail.co')).toBe('ada@gmail.com');
    expect(suggestEmail('ada@gmail.con')).toBe('ada@gmail.com');
    expect(suggestEmail('ada@gmail')).toBe('ada@gmail.com');
    expect(suggestEmail('ada@qq.con')).toBe('ada@qq.com');
    expect(suggestEmail('ada@qq')).toBe('ada@qq.com');
    expect(suggestEmail('ada@163.cmo')).toBe('ada@163.com');
    expect(suggestEmail('ada@icloud.cm')).toBe('ada@icloud.com');
    expect(suggestEmail(' Ada.Lovelace+x@Hotmial.com ')).toBe('Ada.Lovelace+x@hotmail.com');
  });

  it('各国域名都有的邮箱只改名字', () => {
    expect(suggestEmail('ada@hotmial.co.uk')).toBe('ada@hotmail.co.uk');
    expect(suggestEmail('ada@yahooo.co.jp')).toBe('ada@yahoo.co.jp');
    expect(suggestEmail('ada@outlok.fr')).toBe('ada@outlook.fr');
  });

  it('写对的地址和别家的真邮箱不提示', () => {
    for (const address of [
      'ada@gmail.com',
      'ada@qq.com',
      'ada@yahoo.co.jp',
      'ada@hotmail.co.uk',
      'ada@protonmail.ch',
      'ada@email.com',
      'ada@ymail.com',
      'ada@mail.ru',
      'ada@me.com',
      'ada@mi.com',
      'ada@live.cn',
      'ada@proton.me',
      'ada@agent-room.test',
      'ada@example.com',
    ])
      expect(suggestEmail(address), address).toBeNull();
  });

  it('还不是完整地址时不猜', () => {
    for (const value of ['', 'ada', 'ada@', '@gmail.com', 'ada@g', 'ada@mail'])
      expect(suggestEmail(value), value).toBeNull();
  });
});
