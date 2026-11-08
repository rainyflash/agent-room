/* global document, HTMLInputElement, HTMLElement -- 身份服务的注册页在浏览器里加载它，测试在 Node 里只用 suggestEmail。 */
// 注册页：邮箱像是常见邮箱打错了（gmial.com、gmail.comm）时，在输入框下面问一句“是不是想写 …”。
// 只提示、不拦：人可以留着自己写的。打错的地址收不到验证信，这个人就停在验证那一步（2026-11-08 生产上见过 gmail.comm）。

// 只有一个域名的邮箱：名字后面那段打错了也能改回去。
const SINGLE_DOMAIN = {
  gmail: 'gmail.com',
  googlemail: 'googlemail.com',
  qq: 'qq.com',
  163: '163.com',
  126: '126.com',
  foxmail: 'foxmail.com',
  icloud: 'icloud.com',
  yeah: 'yeah.net',
  aliyun: 'aliyun.com',
};

// 各国域名都有的邮箱（hotmail.co.uk、yahoo.co.jp）：只改名字，后面那段原样留着。
const MANY_DOMAINS = ['hotmail', 'outlook', 'yahoo', 'protonmail'];

// 跟上面的名字只差一两个字、却是别家真邮箱的名字，别去“纠正”。
const OTHER_MAILBOXES = new Set([
  'email',
  'mail',
  'ymail',
  'gmx',
  'aol',
  'zoho',
  'yandex',
  'web',
  'hey',
  'pm',
  'sina',
  'sohu',
  'tom',
  '139',
  '189',
  'live',
  'msn',
  'me',
  'mac',
  'proton',
  'tutanota',
  'fastmail',
  'naver',
  'daum',
  'hanmail',
]);

const KNOWN_NAMES = [...Object.keys(SINGLE_DOMAIN), ...MANY_DOMAINS];

/** 两个字符串差几个字（增、删、改、相邻两字对调各算一次）。 */
function distance(left, right) {
  const rows = left.length + 1;
  const columns = right.length + 1;
  const table = Array.from({ length: rows }, (_, row) =>
    Array.from({ length: columns }, (_, column) => (row === 0 ? column : column === 0 ? row : 0)),
  );
  for (let row = 1; row < rows; row += 1) {
    for (let column = 1; column < columns; column += 1) {
      const cost = left[row - 1] === right[column - 1] ? 0 : 1;
      let best = Math.min(
        table[row - 1][column] + 1,
        table[row][column - 1] + 1,
        table[row - 1][column - 1] + cost,
      );
      if (
        row > 1 &&
        column > 1 &&
        left[row - 1] === right[column - 2] &&
        left[row - 2] === right[column - 1]
      )
        best = Math.min(best, table[row - 2][column - 2] + 1);
      table[row][column] = best;
    }
  }
  return table[rows - 1][columns - 1];
}

function closestName(name) {
  if (OTHER_MAILBOXES.has(name)) return null;
  if (KNOWN_NAMES.includes(name)) return name;
  // qq、163 这样的短名字离公司自己的域名太近，猜不准。
  if (name.length < 4) return null;
  let best = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const known of KNOWN_NAMES) {
    if (known.length < 5) continue;
    const gap = distance(name, known);
    if (gap < bestDistance) {
      best = known;
      bestDistance = gap;
    }
  }
  return bestDistance <= (name.length >= 8 ? 2 : 1) ? best : null;
}

function correctedDomain(name, rest) {
  const known = closestName(name);
  if (known === null) return null;
  const single = SINGLE_DOMAIN[known];
  if (single !== undefined) {
    const expected = single.slice(known.length + 1);
    return rest === '' || distance(rest, expected) <= 2 ? single : null;
  }
  return rest === '' ? null : `${known}.${rest}`;
}

/** 像是打错了就给出改好的整个地址，不像就给 null。 */
export function suggestEmail(value) {
  const address = value.trim();
  const at = address.lastIndexOf('@');
  if (at <= 0 || at === address.length - 1) return null;
  const domain = address.slice(at + 1).toLowerCase();
  const dot = domain.indexOf('.');
  const name = dot === -1 ? domain : domain.slice(0, dot);
  const rest = dot === -1 ? '' : domain.slice(dot + 1);
  const fixed = correctedDomain(name, rest);
  return fixed === null || fixed === domain ? null : `${address.slice(0, at)}@${fixed}`;
}

function wire(field, hint) {
  const [before, after] = (hint.dataset.template ?? '').split('{email}');
  const show = () => {
    const suggestion = suggestEmail(field.value);
    hint.replaceChildren();
    hint.hidden = suggestion === null;
    if (suggestion === null) return;
    const strong = document.createElement('strong');
    strong.textContent = suggestion;
    const apply = document.createElement('button');
    apply.type = 'button';
    apply.className = 'ar-email-suggestion__apply';
    apply.textContent = hint.dataset.apply ?? '';
    apply.addEventListener('click', () => {
      field.value = suggestion;
      hint.hidden = true;
      hint.replaceChildren();
      field.focus();
    });
    hint.append(before ?? '', strong, after ?? '', ' ', apply);
  };
  field.addEventListener('change', show);
  field.addEventListener('blur', show);
  // 已经在提示了，边改边看还像不像打错。
  field.addEventListener('input', () => {
    if (!hint.hidden) show();
  });
  // 注册没过、页面带着刚才填的地址回来时也看一眼。
  if (field.value !== '') show();
}

if (typeof document !== 'undefined') {
  const field = document.getElementById('email');
  const hint = document.getElementById('email-suggestion');
  if (field instanceof HTMLInputElement && hint instanceof HTMLElement) wire(field, hint);
}
