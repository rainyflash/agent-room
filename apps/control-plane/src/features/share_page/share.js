/* global document, navigator, HTMLInputElement, Element */
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
