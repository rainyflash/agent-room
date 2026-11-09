import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import '@agent-room/ui-system/styles.css';
import '@/app/styles.css';
import './social-preview-fixture.css';

import { RoomIllustration } from '@/features/lobby/ui/room-illustration';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { resources } from '@/shared/i18n/resources';

/**
 * 分享预览图（`public/social-preview.png`，1280×640）的底稿：链接发到别处时显示这张图，GitHub 仓库的
 * 社交预览也用它。标题取首页的英文说法；图上不写域名，自建的服务也用同一张图。
 * 改了首页说法以后重新生成：`AGENT_ROOM_WRITE_SOCIAL_PREVIEW=1` 跑 `e2e/social-preview.e2e.ts`。
 * 加 `?room=en` 或 `?room=zh-CN` 画公开房间分享出去时的通用图（`public/room-share-preview-<语言>.png`）：
 * 不写具体房间的字，房间名和简介在图下面的 `og:` 标题和简介里。
 */
function SocialPreview() {
  const text = resources.en.translation;
  return (
    <main className="social-preview" id="social-preview">
      <section className="social-preview__copy">
        <div className="social-preview__brand">
          <img alt="" src="/agent-room-mark.svg" />
          <span>Agent Room</span>
        </div>
        <h1>{text['landing.title']}</h1>
        <p>Open source · Self-hostable · Built on Matrix</p>
      </section>
      <section className="social-preview__scene">
        <RoomIllustration populated />
        <p className="social-preview__bubble social-preview__bubble--first">
          Can you run the build on your Mac?
        </p>
        <p className="social-preview__bubble social-preview__bubble--second">
          On it. Pasting the log now.
        </p>
      </section>
    </main>
  );
}

const ROOM_SHARE_WORDING = {
  en: {
    bubbles: ['New here? Say hi.', 'Hi! I brought my agent.'],
    detail: 'People and AI agents talk here.',
    tag: 'Public room',
    title: 'Come and join this room.',
  },
  'zh-CN': {
    bubbles: ['新来的？打个招呼吧', '嗨，我把 Agent 也带来了'],
    detail: '人和 AI Agent 在这里聊天。',
    tag: '公开房间',
    title: '来这个房间一起聊。',
  },
} as const;

function RoomSharePreview({ language }: { readonly language: keyof typeof ROOM_SHARE_WORDING }) {
  const text = ROOM_SHARE_WORDING[language];
  return (
    <main className="social-preview" id="social-preview" lang={language}>
      <section className="social-preview__copy">
        <div className="social-preview__brand">
          <img alt="" src="/agent-room-mark.svg" />
          <span>Agent Room</span>
        </div>
        <p className="social-preview__tag">{text.tag}</p>
        <h1>{text.title}</h1>
        <p>{text.detail}</p>
      </section>
      <section className="social-preview__scene">
        <RoomIllustration populated />
        <p className="social-preview__bubble social-preview__bubble--first">{text.bubbles[0]}</p>
        <p className="social-preview__bubble social-preview__bubble--second">{text.bubbles[1]}</p>
      </section>
    </main>
  );
}

async function bootstrap(): Promise<void> {
  // 插图里的人物要用翻译（无障碍名称）；预览图只出英文版。
  await initializeI18n(window.localStorage, ['en']);
  const root = document.getElementById('root');
  if (root === null) throw new Error('Social preview root is missing.');
  const search = new URLSearchParams(window.location.search);
  const roomLanguage = search.get('room');
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      {roomLanguage === 'en' || roomLanguage === 'zh-CN' ? (
        <RoomSharePreview language={roomLanguage} />
      ) : (
        <SocialPreview />
      )}
    </I18nextProvider>,
  );
}

void bootstrap();
