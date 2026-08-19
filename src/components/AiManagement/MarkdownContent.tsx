import { useEffect, useState } from "react";
import { Skeleton, Typography } from "antd";
import DOMPurify from "dompurify";
import { marked } from "marked";

marked.setOptions({
  gfm: true,
  breaks: true,
});

interface MarkdownContentProps {
  text: string;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export default function MarkdownContent({ text }: MarkdownContentProps) {
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setHtml(null);
    void (async () => {
      try {
        const raw = await marked.parse(text || "_空内容_");
        if (cancelled) return;
        setHtml(DOMPurify.sanitize(raw, {
          FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form"],
          FORBID_ATTR: ["style", "onerror", "onload"],
        }));
      } catch (error: unknown) {
        if (cancelled) return;
        const detail = error instanceof Error ? error.message : String(error);
        setHtml("<p>Markdown 渲染失败：" + escapeHtml(detail) + "</p>");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [text]);

  if (html === null) {
    return <Skeleton active title={false} paragraph={{ rows: 2 }} />;
  }

  if (!text.trim()) {
    return <Typography.Text type="secondary">空内容</Typography.Text>;
  }

  return <div className="ai-markdown" dangerouslySetInnerHTML={{ __html: html }} />;
}
