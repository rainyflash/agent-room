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
  if (window.location.pathname !== '/' || read() !== null) return;
  const preferred = (navigator.languages && navigator.languages[0]) || navigator.language || '';
  if (/^zh\b/iu.test(preferred)) window.location.replace('/zh/');
})();
