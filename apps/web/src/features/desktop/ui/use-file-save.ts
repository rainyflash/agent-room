import { useCallback, useState } from 'react';

import { saveBlob } from '@/shared/browser/save-file';

import { useOptionalDesktopRuntimeController } from './desktop-runtime-provider';

/**
 * 桌面端存一份文件走到哪了。`content` 是存的那一份：用的地方换了内容，上一份的结果就不算了
 * （拿 `content` 比）。浏览器里不记：浏览器自己会说下载到哪了。
 */
export type FileSave =
  | { readonly kind: 'saving'; readonly content: Blob }
  | { readonly kind: 'failed'; readonly content: Blob }
  | {
      readonly kind: 'saved';
      readonly content: Blob;
      /** 存成的文件名；重名时和要的不一样。 */
      readonly fileName: string;
      /** 点了“在文件夹里显示”却没打开。 */
      readonly showFailed: boolean;
    };

export type FileSaver = {
  /**
   * 这里是不是由应用自己存（桌面端）。是的话只存得了 `desktopSavesFile` 认的那几种，
   * 存不了的先告诉人；浏览器里什么都交给浏览器自己的下载。
   */
  readonly native: boolean;
  readonly last: FileSave | null;
  readonly save: (fileName: string, content: Blob) => void;
  /** 在文件夹里指出刚存的那个文件；没有存好的、或者这里指不了时是 `null`。 */
  readonly show: (() => void) | null;
};

/**
 * 页面里要把一份现做的内容存成文件都走这里，别直接用 `saveBlob`。
 *
 * 桌面端的窗口里页面自己的下载靠不住：Windows 上悄悄存进“下载”、窗口里什么也不显示，人以为没反应；
 * Mac 上没有处理下载的地方，直接被取消。所以桌面端交给原生层存，存完由 `FileSaveNote` 说存到哪了；
 * 浏览器里照旧交给浏览器自己的下载。
 */
export function useFileSave(): FileSaver {
  const desktop = useOptionalDesktopRuntimeController();
  const saveFile = desktop?.available === true ? desktop.saveFile : undefined;
  const showSavedFile = desktop?.available === true ? desktop.showSavedFile : undefined;
  const [last, setLast] = useState<FileSave | null>(null);

  const save = useCallback(
    (fileName: string, content: Blob): void => {
      if (saveFile === undefined) {
        saveBlob(content, fileName);
        return;
      }
      const saving: FileSave = { kind: 'saving', content };
      setLast(saving);
      void saveFile(fileName, content).then((savedAs) => {
        // 还没存完又存了别的：只说最后点的那一次。
        setLast((current) => {
          if (current !== saving) return current;
          return savedAs === null
            ? { kind: 'failed', content }
            : { kind: 'saved', content, fileName: savedAs, showFailed: false };
        });
      });
    },
    [saveFile],
  );

  const show =
    last?.kind !== 'saved' || showSavedFile === undefined
      ? null
      : (): void => {
          void showSavedFile(last.fileName).then((opened) => {
            setLast((current) => (current === last ? { ...last, showFailed: !opened } : current));
          });
        };

  return { native: saveFile !== undefined, last, save, show };
}
