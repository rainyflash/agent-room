/**
 * 首页房间里的家具和“你”：和应用场景一样是平面矢量（`public/studio/README.md`）。只用表现属性，
 * 不写 `style`：裸域的内容安全策略不放行内联样式。颜色跟着房间走，暗色下也是白天的配色。
 */
const INK = '#2a2733';

export const DESK = `<svg width="124" height="62" viewBox="0 0 124 62"><rect x="46" y="38" width="32" height="22" rx="6" fill="#8ec5f5" stroke="${INK}" stroke-width="2"/><rect x="2" y="2" width="120" height="40" rx="13" fill="#e8b47c" stroke="${INK}" stroke-width="2"/><circle cx="20" cy="22" r="6" fill="#7fc49a" stroke="${INK}" stroke-width="1.5"/><rect x="44" y="11" width="36" height="20" rx="5" fill="#eeeef6" stroke="${INK}" stroke-width="2"/></svg>`;

export const SOFA = `<svg width="150" height="52" viewBox="0 0 150 52"><rect x="2" y="2" width="146" height="48" rx="14" fill="#86c6a9" stroke="${INK}" stroke-width="2"/><rect x="10" y="36" width="130" height="8" rx="4" fill="#a9dcc4" stroke="${INK}" stroke-width="1.5"/></svg>`;

export const PLANT = `<svg width="30" height="36" viewBox="0 0 30 36"><ellipse cx="10" cy="11" rx="5" ry="10" fill="#7fc49a" stroke="${INK}" stroke-width="1.5"/><ellipse cx="20" cy="11" rx="5" ry="10" fill="#7fc49a" stroke="${INK}" stroke-width="1.5"/><path d="M4 21h22v3a11 11 0 0 1-22 0z" fill="#f28a6e" stroke="${INK}" stroke-width="1.5"/></svg>`;

export const YOU = `<svg class="scene__art" width="66" height="74" viewBox="0 0 66 74" aria-hidden="true"><polygon points="33,2 64,20 64,54 33,72 2,54 2,20" fill="${INK}"/><circle cx="33" cy="37" r="12" fill="#fff"/></svg>`;

export const PHONE = `<svg class="scene__art scene__bob" width="124" height="112" viewBox="0 0 124 112" aria-hidden="true"><rect x="3" y="24" width="118" height="84" rx="16" fill="#fffdf7" stroke="${INK}" stroke-width="2.5"/><rect x="12" y="34" width="100" height="58" rx="8" fill="#247a77"/><circle cx="62" cy="100" r="3.5" fill="${INK}"/><rect x="26" y="46" width="52" height="8" rx="4" fill="#bde8d8"/><rect x="26" y="62" width="72" height="8" rx="4" fill="#fff"/><rect x="26" y="76" width="36" height="8" rx="4" fill="#ffc53d"/></svg>`;
