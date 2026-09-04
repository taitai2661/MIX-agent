import { useRef, useState, type FormEvent } from "react";
import { Button } from "@/components/button";

export function ConfirmModal({
  title,
  message,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className="mcp-install-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <div className="mcp-install-panel card form-grid">
        <h2>{title}</h2>
        <p>{message}</p>
        <div className="form-actions">
          <Button
            onClick={() => {
              onConfirm();
              onCancel();
            }}
          >
            確認
          </Button>
          <Button variant="ghost" type="button" onClick={onCancel}>
            キャンセル
          </Button>
        </div>
      </div>
    </div>
  );
}

export function PromptModal({
  title,
  defaultValue = "",
  onConfirm,
  onCancel,
}: {
  title: string;
  defaultValue?: string;
  onConfirm: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(defaultValue);
  const inputRef = useRef<HTMLInputElement>(null);
  return (
    <div
      className="mcp-install-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <form
        className="mcp-install-panel card form-grid"
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          onConfirm(value);
        }}
      >
        <h2>{title}</h2>
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          autoFocus
        />
        <div className="form-actions">
          <Button type="submit">確定</Button>
          <Button variant="ghost" type="button" onClick={onCancel}>
            キャンセル
          </Button>
        </div>
      </form>
    </div>
  );
}
