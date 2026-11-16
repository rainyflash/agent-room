import { describe, expect, it } from 'vitest';

import { eventFocus, resolveFocus } from '@/features/conversation/domain/message-focus';
import { conversationFixture } from '@/features/conversation/testing/conversation-fixture';

describe('按 Matrix 事件 ID 去看一句', () => {
  const messages = [conversationFixture('m1'), conversationFixture('m2')];

  it('加载了就换成它的消息 ID', () => {
    expect(resolveFocus(eventFocus('$m2'), messages)).toBe('m2');
  });

  it('还没加载到时原样交回，对哪一条都对不上，对话照找不到的那一套往前补', () => {
    expect(resolveFocus(eventFocus('$old'), messages)).toBe('matrix-event:$old');
  });

  it('普通的消息 ID 和没有要看的原样交回', () => {
    expect(resolveFocus('m1', messages)).toBe('m1');
    expect(resolveFocus(null, messages)).toBeNull();
  });
});
