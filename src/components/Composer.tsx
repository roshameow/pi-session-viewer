import React from "react";

export function Composer({
  value,
  onChange,
  running,
  disabled,
  targetName,
  onSend,
  onAbort,
}: {
  value: string;
  onChange: (v: string) => void;
  running: boolean;
  disabled?: boolean;
  targetName?: string;
  onSend: (msg: string) => void;
  onAbort: () => void;
}) {
  const submit = () => {
    const v = value.trim();
    if (!v || running) return;
    onSend(v);
    onChange("");
  };

  return (
    <div className="composer-wrap">
      {targetName && (
        <div className="composer-target" title="This message will be sent to the session below">
          → <span className="composer-target-name">{targetName}</span>
        </div>
      )}
      <div className="composer">
        {running ? (
          <button className="btn abort" onClick={onAbort}>
            ⏹ Abort
          </button>
        ) : (
          <button className="btn send" onClick={submit} disabled={disabled || !value.trim()}>
            Send
          </button>
        )}
        <textarea
          className="composer-input"
          placeholder={
            running
              ? "pi is working; message will queue…"
              : "Continue the conversation. Enter to send, Shift+Enter for newline"
          }
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
        />
        <span className="composer-hint">{running ? "⏳ running" : "Enter ↵"}</span>
      </div>
    </div>
  );
}
