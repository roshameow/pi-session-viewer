import React, { useEffect, useRef, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../api";
import { isLocalFileHref, localFileUrlTransform } from "../localFileLinks";

export type LocalFileLinkScope = {
  sessionPath: string;
  allowed: boolean;
  validate?: () => boolean;
  disabledReason?: string;
};

export const LocalFileLinkContext = React.createContext<LocalFileLinkScope>({
  sessionPath: "",
  allowed: false,
  disabledReason: "Local files are only available in a ready local session.",
});

function MarkdownAnchor({ node: _node, href, children, ...rest }: React.ComponentProps<"a"> & { node?: unknown }) {
  const scope = React.useContext(LocalFileLinkContext);
  const [failure, setFailure] = useState<{ scope: LocalFileLinkScope; href: string; message: string } | null>(null);
  const current = useRef({ scope, href, request: 0 });
  if (current.current.scope !== scope || current.current.href !== href) {
    current.current = { scope, href, request: current.current.request + 1 };
  }
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; current.current.request++; };
  }, []);

  if (!href || !isLocalFileHref(href)) {
    return <a {...rest} href={href || undefined} target="_blank" rel="noreferrer">{children}</a>;
  }

  const native = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const unavailable = !scope.allowed || !scope.sessionPath
    ? scope.disabledReason || "Local files are unavailable for remote or not-ready sessions."
    : !native ? "Opening local files requires the desktop app; browser/demo mode cannot open them." : null;
  const open = async (event: React.MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    // Stale event handlers must not open after a context/source change.
    if (!mounted.current || current.current.scope !== scope || current.current.href !== href) return;
    const request = ++current.current.request;
    setFailure(null);
    if (scope.validate && !scope.validate()) {
      setFailure({ scope, href, message: "Session/source changed; choose a ready local session before opening this file." });
      return;
    }
    if (unavailable) {
      setFailure({ scope, href, message: unavailable });
      return;
    }
    try {
      // Click-only IPC. The backend reads the actual session header cwd and
      // rejects remote state; the frontend never probes or resolves paths.
      await api.openLocalFile(href, scope.sessionPath);
    } catch (error) {
      // A native open already started cannot be undone, but a late result must
      // not propagate into another session, preview, or remote source.
      if (mounted.current && current.current.scope === scope && current.current.href === href &&
          current.current.request === request) {
        setFailure({ scope, href, message: "Could not open local file: " + String(error) });
      }
    }
  };
  const error = failure?.scope === scope && failure.href === href ? failure.message : null;
  return <>
    <a {...rest} href={href} onClick={open} onAuxClick={(event) => event.preventDefault()}
      aria-disabled={!!unavailable}
      title={unavailable || `Open local file in its default app (relative paths use this session's working directory): ${href}`}>
      {children}
    </a>
    {error && <span className="md-link-error" role="alert"> {error}</span>}
  </>;
}

// Stable renderer identities keep link state and memoized rows intact. Only
// anchors consume context, so a source change does not reparse Markdown.
const components: Components = {
  a: MarkdownAnchor,
  table: (p) => (
    <div className="md-table-wrap">
      <table {...p} />
    </div>
  ),
  code: ({ className, children, ...rest }) => {
    const isBlock = /language-/.test(className || "");
    if (isBlock) {
      return <code className="md-code-block" {...rest}>{children}</code>;
    }
    return <code className="md-code-inline" {...rest}>{children}</code>;
  },
};

export function Markdown({ text }: { text: string }) {
  // Unchanged text blocks skip react-markdown entirely via MemoMarkdown.
  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components} urlTransform={localFileUrlTransform}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

export const MemoMarkdown = React.memo(Markdown);
