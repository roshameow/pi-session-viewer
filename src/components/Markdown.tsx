import React from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

// Markdown renderer styled after the no-tool-bg / minimal-mode palette.

export function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: (p) => <a {...p} target="_blank" rel="noreferrer" />,
          table: (p) => (
            <div className="md-table-wrap">
              <table {...p} />
            </div>
          ),
          code: ({ className, children, ...rest }) => {
            const isBlock = /language-/.test(className || "");
            if (isBlock) {
              return (
                <code className="md-code-block" {...rest}>
                  {children}
                </code>
              );
            }
            return (
              <code className="md-code-inline" {...rest}>
                {children}
              </code>
            );
          },
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
