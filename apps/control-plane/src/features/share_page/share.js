/* global document, navigator, HTMLInputElement, Element */
// 分享页的“复制链接”。没有脚本时链接照样在输入框里，自己选中复制。复制不了（有的应用内浏览器不给）
// 就选中输入框里的链接，让人自己复制。
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
    const status = document.querySelector('[data-copy-status]');
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
