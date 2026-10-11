import { defaultUrlTransform, type UrlTransform } from "react-markdown";

// Classification only: filesystem resolution/validation belongs to the backend.
// Never infer a path from link text, the session filename, or the browser URL.
export function isLocalFileHref(href: string): boolean {
  if (!href || /[\u0000-\u0020\u007f\\]/.test(href) || href.startsWith("//") ||
      href.startsWith("#") || href.startsWith("?")) return false;
  if (/^file:/i.test(href)) {
    // No network authorities, credentials, ports, or opaque file: URLs.
    if (!/^file:\/\/(?:localhost)?\//i.test(href)) return false;
    try {
      const url = new URL(href);
      return url.protocol === "file:" && (!url.hostname || url.hostname === "localhost") &&
        !url.username && !url.password && !url.port && url.pathname.startsWith("/") &&
        !url.pathname.startsWith("//");
    } catch {
      return false;
    }
  }
  // File-like bare names may carry numeric line/column decorations. This is
  // not a general URI exception: javascript:12 and arbitrary schemes stay blocked.
  if (/^[^/:?#]+\.[A-Za-z0-9]+:\d+(?::\d+)?(?:#L\d+(?:C\d+)?(?:-L\d+(?:C\d+)?)?)?$/.test(href)) return true;
  return !/^[^/?#]*:/.test(href);
}

export const localFileUrlTransform: UrlTransform = (url, key, node) => {
  // Only anchors get the narrow file: exception. In particular, file: image
  // sources retain react-markdown's default sanitization and cannot load files.
  if (url.startsWith("//")) return "";
  if (key === "href" && node.tagName === "a" && isLocalFileHref(url)) return url;
  return defaultUrlTransform(url);
};
