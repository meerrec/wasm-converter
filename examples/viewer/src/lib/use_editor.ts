import { useCallback, useEffect, useRef, useState } from 'react';
import { OoxmlEditor } from '../editor/client.js';
import type { EditorArgs, EditorFormat, EditorResult } from '../editor/types.js';

/** Жизненный цикл editor-worker и признаки истории для быстрых команд ленты. */
export function useEditor(): {
  open(format: EditorFormat, bytes: ArrayBuffer, name: string): Promise<void>;
  create(format: EditorFormat, bytes: ArrayBuffer, name: string): Promise<void>;
  execute(command: string, args?: EditorArgs): Promise<EditorResult>;
  undo(): Promise<EditorResult>;
  redo(): Promise<EditorResult>;
  save(): Promise<EditorResult>;
  canUndo: boolean;
  canRedo: boolean;
} {
  const ref = useRef<OoxmlEditor | null>(null);
  const [history, setHistory] = useState({ canUndo: false, canRedo: false });
  const editor = useCallback((): OoxmlEditor => {
    ref.current ??= new OoxmlEditor();
    return ref.current;
  }, []);
  const update = useCallback((result: EditorResult): EditorResult => {
    setHistory({ canUndo: result.canUndo, canRedo: result.canRedo });
    return result;
  }, []);

  useEffect(() => () => {
    ref.current?.destroy();
    ref.current = null;
  }, []);

  return {
    open: useCallback(async (format, bytes, name) => {
      update(await editor().open(format, bytes, name));
    }, [editor, update]),
    create: useCallback(async (format, bytes, name) => {
      update(await editor().create(format, bytes, name));
    }, [editor, update]),
    execute: useCallback(async (command, args = {}) => update(await editor().execute(command, args)), [editor, update]),
    undo: useCallback(async () => update(await editor().undo()), [editor, update]),
    redo: useCallback(async () => update(await editor().redo()), [editor, update]),
    save: useCallback(() => editor().save(), [editor]),
    canUndo: history.canUndo,
    canRedo: history.canRedo,
  };
}
