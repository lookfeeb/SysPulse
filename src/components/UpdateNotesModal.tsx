import { useEffect, useMemo, useState } from "react";
import { Modal, Skeleton, Tag, Typography } from "antd";
import { CloudDownloadOutlined } from "@ant-design/icons";
import { marked } from "marked";
import DOMPurify from "dompurify";

const { Text } = Typography;

// Configure marked once: GFM on, line breaks as <br>, no HTML passthrough —
// everything is then re-sanitized by DOMPurify below.
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
  onClose: () => void;
}

export default function UpdateNotesModal({
  open,
  version,
  currentVersion,
  notes,
  loading = false,
  onClose,
}: Props) {
  // Render async so large notes don't block the modal opening animation.
  const [html, setHtml] = useState<string | null>(null);

  const trimmed = useMemo(() => notes.trim(), [notes]);

  useEffect(() => {
    if (!open || loading) {
      setHtml(null);
      return;
    }
    let cancelled = false;
    setHtml(null);
    (async () => {
      try {
        const raw = await marked.parse(trimmed || "_暂无更新说明_");
        if (cancelled) return;
        const clean = DOMPurify.sanitize(raw, {
          // Keep typical markdown output, forbid scripts / embeds / event handlers.
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
      open={open}
      onCancel={onClose}
      onOk={onClose}
      title={
        <span>
          <CloudDownloadOutlined style={{ marginRight: 8, color: "#3388cc" }} />
          {version === currentVersion ? `v${version} 更新日志` : `新版本 v${version}`}
          {version !== currentVersion && (
            <Tag color="default" style={{ marginLeft: 10, fontWeight: 400 }}>
              当前 v{currentVersion}
            </Tag>
          )}
        </span>
      }
      width={640}
      footer={null}
      styles={{ body: { padding: "12px 20px 20px" } }}
      destroyOnHidden
    >
      {html === null ? (
        <Skeleton active paragraph={{ rows: 4 }} title={false} />
      ) : (
        <div
          className="update-notes"
          // html is `marked` output -> DOMPurify sanitized. Safe to inject.
          dangerouslySetInnerHTML={{ __html: html }}
        />
      )}
      {!trimmed && html !== null && (
        <Text type="secondary" style={{ fontSize: 12 }}>
          此版本发行说明为空。
        </Text>
      )}
    </Modal>
  );
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
