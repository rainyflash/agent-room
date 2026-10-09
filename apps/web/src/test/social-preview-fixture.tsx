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

async function bootstrap(): Promise<void> {
  // 插图里的人物要用翻译（无障碍名称）；预览图只出英文版。
  await initializeI18n(window.localStorage, ['en']);
  const root = document.getElementById('root');
  if (root === null) throw new Error('Social preview root is missing.');
  createRoot(root).render(
    <I18nextProvider i18n={i18n}>
      <SocialPreview />
    </I18nextProvider>,
  );
}

void bootstrap();
