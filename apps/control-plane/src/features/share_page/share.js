/* global document, navigator, window, fetch, HTMLInputElement, Element */
// 分享页的“复制链接”，仓库地址那一页的“复制命令”也是它。没有脚本时字照样在输入框里，自己选中复制。
// 复制不了（有的应用内浏览器不给）就选中输入框里的字，让人自己复制。一页上有几个按钮时，结果写在按钮
// 所在的 data-copy-scope 里那一行。
(() => {
  const select = (input) => {
    if (!(input instanceof HTMLInputElement)) return;
    input.focus();
    input.select();
  };
  document.addEventListener('click', (event) => {
    const button =
      event.target instanceof Element ? event.target.closest('[data-copy-link]') : null;
    if (button === null) return;
    const input = document.getElementById(button.getAttribute('data-copy-link') || '');
    const scope = button.closest('[data-copy-scope]') || document;
    const status = scope.querySelector('[data-copy-status]');
    const say = (text) => {
      if (status !== null) status.textContent = text || '';
    };
    if (!(input instanceof HTMLInputElement)) return;
    if (!navigator.clipboard || !navigator.clipboard.writeText) {
      select(input);
      say(button.getAttribute('data-select'));
      return;
    }
    navigator.clipboard.writeText(input.value).then(
      () => say(button.getAttribute('data-copied')),
      () => {
        select(input);
        say(button.getAttribute('data-select'));
      },
    );
  });
})();

// 分享页的实况：页面在前台时每 5 秒问一次 /r/<码>/live，切到后台就停。头一次问到才露出那一块；
// 回 404（不放了）就藏起来、不再问；暂时问不到时留着上一次的。房间里的话和名字都是别人写的，
// 一律当纯文本放（textContent），不拼 HTML。
(() => {
  const panel = document.querySelector('[data-live]');
  if (panel === null || typeof fetch !== 'function') return;
  const url = panel.getAttribute('data-live') || '';
  const list = panel.querySelector('[data-live-messages]');
  const status = panel.querySelector('[data-live-status]');
  const empty = panel.querySelector('[data-live-empty]');
  const word = (name) => panel.getAttribute(`data-${name}`) || '';
  const language = document.documentElement.lang || undefined;
  const intervalMs = 5000;
  let timer = null;
  let stopped = false;

  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  };

  const time = (unixMs) => {
    try {
      return new Date(unixMs).toLocaleTimeString(language, { hour: '2-digit', minute: '2-digit' });
    } catch {
      return '';
    }
  };

  const kind = (participant) => {
    if (participant.kind === 'networkAgent') return word('network-agent');
    if (participant.kind !== 'agent') return '';
    if (typeof participant.owner === 'string' && participant.owner !== '') {
      return word('agent-of').replace('{owner}', participant.owner);
    }
    return word('agent');
  };

  const message = (item, participants) => {
    const author = participants.get(item.author);
    const row = element('li', 'share-live-message');
    const meta = element('div', 'share-live-meta');
    meta.append(element('span', 'share-live-name', author ? author.name : ''));
    const label = author ? kind(author) : '';
    if (label) meta.append(element('span', 'share-live-kind', label));
    const when = element('time', 'share-live-time', time(item.sentAtUnixMs));
    when.dateTime = new Date(item.sentAtUnixMs).toISOString();
    meta.append(when);
    row.append(meta);
    if (item.withheld) {
      row.append(element('p', 'share-live-text share-live-text--quiet', word('withheld')));
      return row;
    }
    if (typeof item.text === 'string' && item.text !== '') {
      let text = item.text;
      if (item.truncated) text += word('truncated');
      if (item.edited) text += word('edited');
      row.append(element('p', 'share-live-text', text));
    }
    if (item.attachment) {
      row.append(element('p', 'share-live-text share-live-text--quiet', word('file')));
    }
    return row;
  };

  const show = (view) => {
    if (view === null || typeof view !== 'object' || view.schemaVersion !== 1) return;
    const participants = new Map();
    for (const participant of Array.isArray(view.participants) ? view.participants : []) {
      participants.set(participant.key, participant);
    }
    const messages = Array.isArray(view.messages) ? view.messages : [];
    if (list !== null) list.replaceChildren(...messages.map((item) => message(item, participants)));
    if (empty !== null) empty.hidden = messages.length > 0;
    if (status !== null) {
      const online = view.online;
      status.textContent =
        online && typeof online.count === 'number'
          ? word(online.atLeast ? 'online-at-least' : 'online').replace(
              '{count}',
              String(online.count),
            )
          : word('read-only');
    }
    panel.hidden = false;
  };

  const schedule = () => {
    if (stopped || timer !== null || document.hidden) return;
    timer = window.setTimeout(() => {
      timer = null;
      load();
    }, intervalMs);
  };

  const load = () => {
    if (stopped || document.hidden) return;
    fetch(url, { headers: { Accept: 'application/json' }, credentials: 'omit' })
      .then((response) => {
        if (response.status === 404) {
          stopped = true;
          panel.hidden = true;
          return null;
        }
        return response.ok ? response.json() : null;
      })
      .then(show)
      .catch(() => undefined)
      .finally(schedule);
  };

  document.addEventListener('visibilitychange', () => {
    if (document.hidden) {
      if (timer !== null) window.clearTimeout(timer);
      timer = null;
      return;
    }
    load();
  });
  load();
})();
