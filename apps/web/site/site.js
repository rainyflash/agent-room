/* global window, document, navigator, Element, HTMLElement */
// agentroom.chat 的语言：浏览器首选中文、又没在网站上选过语言时，英文首页跳到中文首页。
// 爬虫的首选语言是英文，不会被跳走。点了页面上的语言链接就记下来，以后不再跳。
(() => {
  const key = 'agent-room-site-language';
  const read = () => {
    try {
      return window.localStorage.getItem(key);
    } catch {
      return null;
    }
  };
  const remember = (language) => {
    try {
      window.localStorage.setItem(key, language);
    } catch {
      // 存不了就每次按浏览器的语言来。
    }
  };
  document.addEventListener('click', (event) => {
    const link = event.target instanceof Element ? event.target.closest('a[data-language]') : null;
    if (link instanceof HTMLElement && link.dataset.language) remember(link.dataset.language);
  });
  // 下载按钮默认指向页面上写着两个安装包的那一行；认得出系统就直接给这个系统的安装包。
  document.addEventListener('DOMContentLoaded', () => {
    const agent = navigator.userAgent;
    const windows = /Windows NT/u.test(agent);
    const mac = /Macintosh/u.test(agent) && navigator.maxTouchPoints <= 1;
    if (!windows && !mac) return;
    for (const link of document.querySelectorAll('a[data-windows][data-macos]')) {
      if (link instanceof HTMLElement) {
        link.setAttribute('href', (windows ? link.dataset.windows : link.dataset.macos) ?? '');
      }
    }
  });
  if (window.location.pathname !== '/' || read() !== null) return;
  const preferred = (navigator.languages && navigator.languages[0]) || navigator.language || '';
  if (/^zh\b/iu.test(preferred)) window.location.replace('/zh/');
})();
