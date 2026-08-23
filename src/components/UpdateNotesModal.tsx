import { useEffect, useMemo, useState } from "react";
import { Modal, Skeleton } from "antd";
import {
  CloudDownloadOutlined,
  FileTextOutlined,
  TagOutlined,
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
      open={open}
      onCancel={onClose}
      onOk={onClose}
      title={null}
      width={640}
      footer={null}
      styles={{
        body: { padding: 0 },
        container: { borderRadius: 14, overflow: "hidden" },
      }}
      destroyOnHidden
    >
      <header className="unm-head" data-variant={isCurrent ? "current" : "update"}>
        <div className="unm-head-row">
          <div className="unm-mark">
            {isCurrent ? <FileTextOutlined /> : <CloudDownloadOutlined />}
          </div>
          <div className="unm-head-text">
            <h2 className="unm-title">
              {isCurrent ? "当前版本更新日志" : "发现新版本"}
            </h2>
            <div className="unm-meta">
              <span className="unm-ver">
                <TagOutlined />
                v{version}
              </span>
              {!isCurrent && (
                <span className="unm-ver unm-ver--old">当前 v{currentVersion}</span>
              )}
              {publishDate && (
                <span className="unm-time">
                  <ClockCircleOutlined />
                  {publishDate}
                </span>
              )}
            </div>
          </div>
        </div>
      </header>

      <div className="unm-body">
        {html === null ? (
          <Skeleton active paragraph={{ rows: 5 }} title={false} />
        ) : trimmed ? (
          <>
            <div className="unm-label">
              更新内容
              <i aria-hidden="true" />
            </div>
            <div
              className="update-notes"
              dangerouslySetInnerHTML={{ __html: html }}
            />
          </>
        ) : (
          <span className="unm-void">此版本发行说明为空。</span>
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
