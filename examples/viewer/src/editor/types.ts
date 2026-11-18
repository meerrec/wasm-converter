export type EditorFormat = 'docx' | 'xlsx' | 'pptx';
export type EditorArgs = Record<string, unknown>;

export interface EditorResult {
  changed: boolean;
  canUndo: boolean;
  canRedo: boolean;
  bytes?: ArrayBuffer;
  value?: unknown;
  message?: string;
}

export type EditorRequest =
  | { type: 'open'; id: number; format: EditorFormat; bytes: ArrayBuffer; name: string }
  | { type: 'create'; id: number; format: EditorFormat; bytes: ArrayBuffer; name: string }
  | { type: 'execute'; id: number; command: string; args: EditorArgs }
  | { type: 'save'; id: number }
  | { type: 'undo' | 'redo'; id: number }
  | { type: 'close' };

export type EditorResponse =
  | ({ type: 'result'; id: number } & EditorResult)
  | { type: 'error'; id: number; message: string };
