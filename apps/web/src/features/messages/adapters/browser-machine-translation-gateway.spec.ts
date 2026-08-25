import { describe, expect, it, vi } from 'vitest';

import {
  BrowserMachineTranslationGateway,
  type BrowserTranslatorFactory,
} from './browser-machine-translation-gateway';

describe('浏览器机器翻译适配器', () => {
  it('只在显式调用后创建本地翻译器，并永久保留原文与机器来源', async () => {
    const destroy = vi.fn();
    const translate = vi.fn(() => Promise.resolve('你好'));
    const factory = {
      availability: vi.fn(() => Promise.resolve('available' as const)),
      create: vi.fn(() => Promise.resolve({ destroy, translate })),
    } satisfies BrowserTranslatorFactory;
    const gateway = new BrowserMachineTranslationGateway(factory);

    expect(factory.availability).not.toHaveBeenCalled();
    const result = await gateway.translate({
      originalText: 'Hello',
      sourceLanguage: 'en-US',
      targetLanguage: 'zh-CN',
    });

    expect(result).toEqual({
      ok: true,
      value: {
        originalText: 'Hello',
        provenance: 'machine',
        sourceLanguage: 'en',
        targetLanguage: 'zh',
        translatedText: '你好',
      },
    });
    expect(factory.create).toHaveBeenCalledOnce();
    expect(destroy).toHaveBeenCalledOnce();
  });

  it('能力不可用时显式失败，不伪造翻译', async () => {
    const result = await new BrowserMachineTranslationGateway(null).translate({
      originalText: 'Hello',
      sourceLanguage: 'en',
      targetLanguage: 'zh-CN',
    });

    expect(result).toEqual({ ok: false, error: { code: 'unavailable', retryable: false } });
  });
});
