import { useEffect, useState } from "react";
import { Skeleton, Typography } from "antd";
import DOMPurify from "dompurify";
import { Marked } from "marked";

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// 独立实例，不碰 marked 全局单例——UpdateNotesModal 用的是全局的那个。
//
// AI 会话正文里经常整段贴着 HTML（页面结构、组件片段等）。默认 marked 会原样
// 透传，dangerouslySetInnerHTML 就把别人的页面渲染进了消息气泡里，布局直接被撑爆。
// 这里把块级和内联 HTML 都按字面转成代码，既忠实还原原文，也杜绝结构注入。
const sessionMarkdown = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    html({ text, block }) {
      const escaped = escapeHtml(text.replace(/\n+$/, ""));
      return block
        ? "<pre><code>" + escaped + "</code></pre>"
        : "<code>" + escaped + "</code>";
    },
  },
});

interface MarkdownContentProps {
  text: string;
}

export default function MarkdownContent({ text }: MarkdownContentProps) {
  const [html, setHtml] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setHtml(null);
    void (async () => {
      try {
        const raw = await sessionMarkdown.parse(text || "_空内容_");
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
