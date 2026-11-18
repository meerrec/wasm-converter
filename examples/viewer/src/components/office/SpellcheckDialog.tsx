import { useState, type ReactElement } from 'react';
import { OfficeDialog } from './OfficeDialog.js';

/** Нативная проверка браузера: исправленный текст применяется только по подтверждению. */
export function SpellcheckDialog(props: {
  text: string;
  onClose(): void;
  onApply(text: string): Promise<void>;
}): ReactElement {
  const [text, setText] = useState(props.text);
  const [busy, setBusy] = useState(false);
  return (
    <OfficeDialog
      title="Правописание"
      onClose={props.onClose}
      actions={
        <button
          type="button"
          className="ofc-primary"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void props.onApply(text).finally(() => setBusy(false));
          }}
        >Применить исправления</button>
      }
    >
      <label className="ofc-dialog__field">
        <span>Текст с подсветкой ошибок браузера</span>
        <textarea
          className="ofc-spellcheck"
          value={text}
          spellCheck
          autoFocus
          lang={navigator.language}
          onChange={(event) => setText(event.currentTarget.value)}
        />
      </label>
    </OfficeDialog>
  );
}
