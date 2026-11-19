// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { useOptionalAppServices } from '@/app/app-services';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import type { ContentFailure, DownloadedContent } from '@/features/messages/domain/content';
import type { MessageContentReference } from '@/features/messages/domain/message';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';
import { err, ok, type Result } from '@/shared/result';

import { MessageAttachment } from './message-attachment';

vi.mock('@/app/app-services', () => ({ useOptionalAppServices: vi.fn(() => null) }));
vi.mock('@/features/desktop/ui/desktop-runtime-provider', () => ({
  useOptionalDesktopRuntimeController: vi.fn(() => null),
}));

type SaveFile = (fileName: string, content: Blob) => Promise<string | null>;
type ShowSavedFile = (fileName: string) => Promise<boolean>;

const bytes = new TextEncoder().encode('canary 4821');

function reference(mediaType: string): MessageContentReference {
  return { contentId: 'content-1', digestSha256: 'ab'.repeat(32), mediaType, sizeBytes: 11 };
}

const download = vi.fn<() => Promise<Result<DownloadedContent, ContentFailure>>>();

function useServices() {
  vi.mocked(useOptionalAppServices).mockReturnValue({
    content: {
      issueReadTicket: () => Promise.resolve(ok({ expiresAtUnixMs: 1, ticket: 'ticket' })),
      download,
    },
    contentVerifier: {
      verify: (downloaded: DownloadedContent, expected: MessageContentReference) =>
        Promise.resolve(
          ok({
            bytes: downloaded.bytes,
            digestSha256: expected.digestSha256,
            mediaType: expected.mediaType,
            mode: 'download' as const,
          }),
        ),
    },
  } as unknown as ReturnType<typeof useOptionalAppServices>);
}

function useDesktop(
  saveFile: SaveFile,
  showSavedFile: ShowSavedFile = () => Promise.resolve(true),
) {
  vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue({
    available: true,
    saveFile,
    showSavedFile,
  } as unknown as ReturnType<typeof useOptionalDesktopRuntimeController>);
}

function renderAttachment(name: string, mediaType: string) {
  return render(
    <I18nextProvider i18n={i18n}>
      <MessageAttachment content={reference(mediaType)} name={name} roomId="!room:test" />
    </I18nextProvider>,
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

beforeEach(() => {
  download.mockResolvedValue(ok({ bytes, contentDigest: null, mediaType: null }));
  useServices();
  URL.createObjectURL = vi.fn(() => 'blob:attachment');
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.mocked(useOptionalAppServices).mockReturnValue(null);
  vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue(null);
});

describe('消息里的附件', () => {
  it('浏览器里：先取回来，再给一个下载链接，交给浏览器自己的下载', async () => {
    renderAttachment('notes.txt', 'text/plain');

    fireEvent.click(screen.getByRole('button', { name: 'Load file' }));

    const link = await screen.findByRole('link', { name: 'Download file' });
    expect(link).toHaveAttribute('href', 'blob:attachment');
    expect(link).toHaveAttribute('download', 'notes.txt');
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('浏览器里什么类型都给下载链接', async () => {
    renderAttachment('bundle.zip', 'application/zip');

    fireEvent.click(screen.getByRole('button', { name: 'Load file' }));

    expect(await screen.findByRole('link', { name: 'Download file' })).toHaveAttribute(
      'download',
      'bundle.zip',
    );
  });

  it('桌面端：点一次“下载文件”就取回来、由应用存好，说存到哪了，能在文件夹里指出来', async () => {
    const saveFile = vi.fn<SaveFile>(() => Promise.resolve('notes (1).txt'));
    const showSavedFile = vi.fn<ShowSavedFile>(() => Promise.resolve(true));
    useDesktop(saveFile, showSavedFile);
    renderAttachment('notes.txt', 'text/plain');

    fireEvent.click(screen.getByRole('button', { name: 'Download file' }));

    expect(await screen.findByRole('status')).toHaveTextContent(
      'Saved to your Downloads folder as notes (1).txt',
    );
    const [fileName, saved] = saveFile.mock.calls[0] ?? [];
    expect(fileName).toBe('notes.txt');
    expect(await saved?.text()).toBe('canary 4821');
    // 页面自己的下载链接在桌面端的窗口里不出声，不能留着。
    expect(screen.queryByRole('link')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Show in folder' }));
    expect(showSavedFile).toHaveBeenCalledWith('notes (1).txt');

    // 再点一次存的是已经取回来的那一份，不再问服务器。
    fireEvent.click(screen.getByRole('button', { name: 'Download file' }));
    await vi.waitFor(() => {
      expect(saveFile).toHaveBeenCalledTimes(2);
    });
    expect(download).toHaveBeenCalledTimes(1);
  });

  it('桌面端的图片：先看，再点“下载文件”才存', async () => {
    const saveFile = vi.fn<SaveFile>(() => Promise.resolve('diagram.png'));
    useDesktop(saveFile);
    renderAttachment('diagram.png', 'image/png');

    fireEvent.click(screen.getByRole('button', { name: 'View image' }));

    expect(await screen.findByRole('img', { name: 'diagram.png' })).toHaveAttribute(
      'src',
      'blob:attachment',
    );
    expect(saveFile).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Download file' }));
    expect(await screen.findByRole('status')).toHaveTextContent(
      'Saved to your Downloads folder as diagram.png',
    );
  });

  it('桌面端存不了的类型：照实说，不给按钮，也不去取', () => {
    const saveFile = vi.fn<SaveFile>();
    useDesktop(saveFile);
    renderAttachment('bundle.zip', 'application/zip');

    expect(
      screen.getByText(
        'The desktop app only saves text, image and PDF files. To download this one, open Agent Room in your browser.',
      ),
    ).toBeVisible();
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.queryByRole('link')).toBeNull();
    expect(download).not.toHaveBeenCalled();
  });

  it('桌面端名字不带能存的扩展名的图片：照样能看，存不了照实说', async () => {
    useDesktop(vi.fn<SaveFile>());
    renderAttachment('diagram', 'image/png');

    fireEvent.click(screen.getByRole('button', { name: 'View image' }));

    expect(await screen.findByRole('img', { name: 'diagram' })).toBeVisible();
    expect(screen.getByText(/only saves text, image and PDF files/u)).toBeVisible();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('桌面端没取回来：说打不开，能重试；重试取回来了接着存', async () => {
    const saveFile = vi.fn<SaveFile>(() => Promise.resolve('notes.txt'));
    useDesktop(saveFile);
    download.mockResolvedValueOnce(err({ code: 'content.offline', retryable: true }));
    renderAttachment('notes.txt', 'text/plain');

    fireEvent.click(screen.getByRole('button', { name: 'Download file' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The attachment could not be opened.',
    );
    expect(saveFile).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByRole('status')).toHaveTextContent(
      'Saved to your Downloads folder as notes.txt',
    );
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('桌面端没存成：说一句，再点一次接着存', async () => {
    const saveFile = vi
      .fn<SaveFile>()
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce('notes.txt');
    useDesktop(saveFile);
    renderAttachment('notes.txt', 'text/plain');

    fireEvent.click(screen.getByRole('button', { name: 'Download file' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('Couldn’t save the file.');
    fireEvent.click(screen.getByRole('button', { name: 'Download file' }));
    expect(await screen.findByRole('status')).toHaveTextContent(
      'Saved to your Downloads folder as notes.txt',
    );
  });
});
