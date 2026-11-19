// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { I18nextProvider } from 'react-i18next';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { saveBlob } from '@/shared/browser/save-file';
import { i18n, initializeI18n } from '@/shared/i18n/i18n';

import { useOptionalDesktopRuntimeController } from './desktop-runtime-provider';
import { FileSaveNote } from './file-save-note';
import { useFileSave } from './use-file-save';

vi.mock('./desktop-runtime-provider', () => ({
  useOptionalDesktopRuntimeController: vi.fn(() => null),
}));
vi.mock('@/shared/browser/save-file', () => ({ saveBlob: vi.fn() }));

type Controller = ReturnType<typeof useOptionalDesktopRuntimeController>;
type SaveFile = (fileName: string, content: Blob) => Promise<string | null>;
type ShowSavedFile = (fileName: string) => Promise<boolean>;

const card = new Blob(['png'], { type: 'image/png' });
const other = new Blob(['{}'], { type: 'application/json' });

function desktop(commands: {
  readonly available?: boolean;
  readonly saveFile?: SaveFile;
  readonly showSavedFile?: ShowSavedFile;
}): void {
  vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue({
    available: true,
    ...commands,
  } as unknown as Controller);
}

/** 一个存两份文件的页面：按钮各存一份，下面是存完的那一句。 */
function Page() {
  const saver = useFileSave();
  return (
    <I18nextProvider i18n={i18n}>
      <button
        onClick={() => {
          saver.save('card.png', card);
        }}
        type="button"
      >
        Save card
      </button>
      <button
        onClick={() => {
          saver.save('data.json', other);
        }}
        type="button"
      >
        Save data
      </button>
      <span data-testid="kind">{saver.last?.kind ?? 'idle'}</span>
      <FileSaveNote onShow={saver.show} save={saver.last} />
    </I18nextProvider>
  );
}

beforeAll(async () => {
  await initializeI18n(window.localStorage, ['en']);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.mocked(useOptionalDesktopRuntimeController).mockReturnValue(null);
});

describe('把现做的内容存成文件', () => {
  it('浏览器里交给浏览器自己的下载，页面上不另说', () => {
    render(<Page />);

    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));

    expect(saveBlob).toHaveBeenCalledWith(card, 'card.png');
    expect(screen.getByTestId('kind')).toHaveTextContent('idle');
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Show in folder' })).toBeNull();
  });

  it('不是桌面端的窗口（运行时不可用）时也交给浏览器', () => {
    const saveFile = vi.fn<SaveFile>();
    desktop({ available: false, saveFile });
    render(<Page />);

    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));

    expect(saveBlob).toHaveBeenCalledWith(card, 'card.png');
    expect(saveFile).not.toHaveBeenCalled();
  });

  it('桌面端由原生层存：说存成了什么名字，能在文件夹里指出来', async () => {
    const saveFile = vi.fn<SaveFile>(() => Promise.resolve('card (1).png'));
    const showSavedFile = vi.fn<ShowSavedFile>(() => Promise.resolve(true));
    desktop({ saveFile, showSavedFile });
    render(<Page />);

    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));

    expect(await screen.findByText('Saved to your Downloads folder as card (1).png')).toBeVisible();
    expect(saveFile).toHaveBeenCalledWith('card.png', card);
    expect(saveBlob).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Show in folder' }));
    expect(showSavedFile).toHaveBeenCalledWith('card (1).png');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('文件夹没打开时说去哪找', async () => {
    desktop({
      saveFile: () => Promise.resolve('card.png'),
      showSavedFile: () => Promise.resolve(false),
    });
    render(<Page />);
    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));

    fireEvent.click(await screen.findByRole('button', { name: 'Show in folder' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Couldn’t open the folder. Look for the file in your Downloads folder.',
    );
    expect(screen.getByText('Saved to your Downloads folder as card.png')).toBeVisible();
  });

  it('没存成就说没存成，不假装存了', async () => {
    desktop({ saveFile: () => Promise.resolve(null) });
    render(<Page />);

    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Couldn’t save the file. Try again.',
    );
    expect(saveBlob).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Show in folder' })).toBeNull();
  });

  it('没有“在文件夹里显示”这条命令时只说存到哪了', async () => {
    desktop({ saveFile: () => Promise.resolve('card.png') });
    render(<Page />);

    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));

    expect(await screen.findByText('Saved to your Downloads folder as card.png')).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Show in folder' })).toBeNull();
  });

  it('前一份还没存完又存了一份：只说后点的那一份', async () => {
    const finish: ((savedAs: string | null) => void)[] = [];
    desktop({
      saveFile: () =>
        new Promise<string | null>((resolve) => {
          finish.push(resolve);
        }),
    });
    render(<Page />);

    fireEvent.click(screen.getByRole('button', { name: 'Save card' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save data' }));
    finish[0]?.('card.png');
    finish[1]?.('data.json');

    expect(await screen.findByText('Saved to your Downloads folder as data.json')).toBeVisible();
    expect(screen.queryByText('Saved to your Downloads folder as card.png')).toBeNull();
  });
});
