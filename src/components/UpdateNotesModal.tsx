import { useEffect, useMemo, useState } from "react";
import { Modal, Skeleton } from "antd";
import {
  CloudDownloadOutlined,
  ClockCircleOutlined,
} from "@ant-design/icons";
import { marked } from "marked";
import DOMPurify from "dompurify";
import "./UpdateNotesModal.css";

marked.setOptions({
  gfm: true,
  breaks: true,
});

interface Props {
  open: boolean;
  version: string;
  currentVersion: string;
  notes: string;
  loading?: boolean;
  publishDate?: string;
  onClose: () => void;
}

/**
 * 发行说明按 `【分类】` 独占一行的约定分组书写，但 Markdown 里它只是普通段落。
 * 先提升成三级标题，正文才能拿到真正的分节层级（顺带去掉方括号）。
 */
function promoteGroupHeadings(source: string): string {
  return source.replace(/^[ \t]*【[ \t]*(.+?)[ \t]*】[ \t]*$/gm, "### $1");
}

export default function UpdateNotesModal({
  open,
  version,
  currentVersion,
  notes,
  loading = false,
  publishDate,
  onClose,
}: Props) {
  const [html, setHtml] = useState<string | null>(null);
  const trimmed = useMemo(() => notes.trim(), [notes]);
  const isCurrent = version === currentVersion;

  useEffect(() => {
    if (!open || loading) {
      setHtml(null);
      return;
    }
    let cancelled = false;
    setHtml(null);
    (async () => {
      try {
        const raw = await marked.parse(
          trimmed ? promoteGroupHeadings(trimmed) : "_暂无更新说明_",
        );
        if (cancelled) return;
        const clean = DOMPurify.sanitize(raw, {
          FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form"],
          FORBID_ATTR: ["style", "onerror", "onload"],
        });
        setHtml(clean);
      } catch (e) {
        if (cancelled) return;
        setHtml(
          `<p>渲染失败：${escapeHtml(
            e instanceof Error ? e.message : String(e),
          )}</p>`,
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, trimmed, loading]);

  return (
    <Modal
      className="unm-modal"
      rootClassName="unm-root"
      open={open}
      onCancel={onClose}
      width={640}
      centered
      closable={{ "aria-label": "关闭" }}
      closeIcon={
        <svg className="unm-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M6 6l12 12M18 6 6 18" />
        </svg>
      }
      footer={
        <button type="button" className="unm-confirm" onClick={onClose}>
          知道了
        </button>
      }
      destroyOnHidden
      title={
        <header className="unm-head">
          <div className="unm-mark" aria-hidden="true">
            {isCurrent ? (
              <svg className="unm-icon" viewBox="0 0 24 24">
                <path d="M14 3H6a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8z" />
                <path d="M14 3v5h5M9 13h6M9 17h4" />
              </svg>
            ) : <CloudDownloadOutlined />}
          </div>
          <div className="unm-head-text">
            <h2 className="unm-title">
              {isCurrent ? "当前版本更新日志" : "发现新版本"}
            </h2>
            <div className="unm-meta">
              <span className="unm-ver">
                <svg className="unm-icon" viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M20.6 13.4 13.4 20.6a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8z" />
                  <circle cx="7.5" cy="7.5" r="1.2" />
                </svg>
                v{version}
              </span>
              <span className="unm-current">
                {isCurrent ? "当前版本" : `当前 v${currentVersion}`}
              </span>
              {publishDate && (
                <span className="unm-time">
                  <ClockCircleOutlined />
                  {publishDate}
                </span>
              )}
            </div>
          </div>
        </header>
      }
    >
      <div className="unm-body" aria-busy={loading || html === null}>
        {loading || html === null ? (
          <div className="unm-loading" role="status" aria-label="正在加载更新日志">
            <Skeleton active paragraph={{ rows: 5 }} title={false} />
          </div>
        ) : trimmed ? (
          <div
            className="unm-notes"
            dangerouslySetInnerHTML={{ __html: html }}
          />
        ) : (
          <div className="unm-empty">
            <svg className="unm-art" viewBox="0 0 120 96" aria-hidden="true">
              <rect className="unm-art-sheet" x="22" y="10" width="76" height="76" rx="10" />
              <path className="unm-art-lines" d="M38 34h44M38 48h44M38 62h24" />
              <circle className="unm-art-badge" cx="92" cy="72" r="16" />
              <path className="unm-art-minus" d="M85 72h14" />
            </svg>
            <strong>此版本发行说明为空</strong>
            <p>作者没有为 v{version} 填写更新内容，暂时没有可显示的变更记录。</p>
          </div>
        )}
      </div>
    </Modal>
  );
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
