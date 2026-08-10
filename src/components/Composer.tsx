import React, { useState } from "react";

export function Composer({
  running,
  disabled,
  onSend,
  onAbort,
}: {
  running: boolean;
  disabled?: boolean;
  onSend: (msg: string) => void;
  onAbort: () => void;
}) {
  const [value, setValue] = useState("");

  const submit = () => {
    const v = value.trim();
    if (!v || running) return;
    onSend(v);
    setValue("");
  };

  return (
    <div className="composer">
      {running ? (
        <button className="btn abort" onClick={onAbort}>
          ⏹ 中止
        </button>
      ) : (
        <button className="btn send" onClick={submit} disabled={disabled}>
          发送
        </button>
      )}
      <textarea
        className="composer-input"
        placeholder={running ? "pi is working; message will queue…" : "Continue the conversation. Enter to send, Shift+Enter for newline"}
        value={value}
        disabled={disabled}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            submit();
          }
        }}
      />
      <span className="composer-hint">{running ? "⏳ running" : "Enter ↵"}</span>
    </div>
  );
}
