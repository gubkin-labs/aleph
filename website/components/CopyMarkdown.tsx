"use client";
import { useState } from "react";

/** Copies the page's Markdown source, for pasting into an AI assistant. */
export function CopyMarkdown({ markdown }: { markdown: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="pill"
      onClick={async () => {
        await navigator.clipboard.writeText(markdown);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    >
      {copied ? "Copied" : "Copy as Markdown"}
    </button>
  );
}
