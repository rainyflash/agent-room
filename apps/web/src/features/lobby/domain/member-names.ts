/**
 * 成员在 Matrix 上的显示名拿来给房间里的人看时写什么。和服务器回答成员名字的规则一样
 * （`crates/application/src/room_people.rs` 的 `shown_display_name`）：带 `@` 的只留 `@` 前面那段，
 * 早期账户的名字可能是邮箱。
 *
 * 认不出名字时回 `null`：没设名字（SDK 这时给的是用户 ID）、名字就是用户名（应用服务刚注册、
 * 还没写上昵称），或者截完是空的。
 */
export function shownMemberName(matrixUserId: string, displayName: string): string | null {
  const name = displayName.trim();
  const colon = matrixUserId.indexOf(':');
  const localpart = matrixUserId.slice(1, colon < 0 ? undefined : colon);
  if (name === matrixUserId || name === localpart) return null;
  const visible = (name.split('@', 1)[0] ?? '').trim();
  return visible === '' ? null : visible;
}
