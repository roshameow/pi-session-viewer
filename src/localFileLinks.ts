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

// Validate without serializing: native opening must receive the exact link URL,
// including an explicit port, trailing slash, query, and fragment.
export function isWebHref(href: string): boolean {
  if (!/^https?:\/\/[^/]/i.test(href) || /[\u0000-\u0020\u007f\\]/.test(href)) return false;
  try {
    const url = new URL(href);
    return (url.protocol === "http:" || url.protocol === "https:") &&
      !!url.hostname && !url.username && !url.password;
  } catch {
    return false;
  }
}

export function isLoopbackWebHref(href: string): boolean {
  if (!isWebHref(href)) return false;
  return ["127.0.0.1", "localhost", "[::1]"].includes(new URL(href).hostname);
}

export const localFileUrlTransform: UrlTransform = (url, key, node) => {
  // Only anchors get the narrow file: exception. In particular, file: image
  // sources retain react-markdown's default sanitization and cannot load files.
  if (url.startsWith("//")) return "";
  if (key === "href" && node.tagName === "a") {
    if (isLocalFileHref(url)) return url;
    if (/^https?:/i.test(url)) {
      // micromark percent-encodes IPv6 authority brackets. Restore only that
      // syntax so URL validation and native opening see the actual HTTP URL;
      // never decode paths, query strings, fragments, or credentials.
      const webUrl = url.replace(/^(https?:\/\/)%5b([a-f0-9:.]+)%5d(?=[:/?#]|$)/i, "$1[$2]");
      return isWebHref(webUrl) ? webUrl : "";
    }
  }
  return defaultUrlTransform(url);
};
