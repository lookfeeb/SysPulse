import { useEffect, useMemo, useState } from "react";
import { Modal, Skeleton, Tag, Typography, Divider } from "antd";
import {
  CloudDownloadOutlined,
  FileTextOutlined,
  TagOutlined,
  ClockCircleOutlined,
} from "@ant-design/icons";
import { marked } from "marked";
import DOMPurify from "dompurify";

const { Text } = Typography;

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
        const raw = await marked.parse(trimmed || "_暂无更新说明_");
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
      open={open}
      onCancel={onClose}
      onOk={onClose}
      title={null}
      width={640}
      footer={null}
      styles={{
        body: { padding: 0 },
        content: { borderRadius: 12, overflow: "hidden" },
      }}
      destroyOnHidden
    >
      {/* Header */}
      <div
        style={{
          background: isCurrent
            ? "linear-gradient(135deg, #f0f9ff 0%, #e0f2fe 100%)"
            : "linear-gradient(135deg, #f0fdf4 0%, #dcfce7 100%)",
          padding: "20px 24px 16px",
          borderBottom: "1px solid #e5e7eb",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <div
            style={{
              width: 36,
              height: 36,
              borderRadius: 10,
              background: isCurrent ? "#3388cc" : "#22c55e",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              flexShrink: 0,
            }}
          >
            {isCurrent ? (
              <FileTextOutlined style={{ fontSize: 16, color: "#fff" }} />
            ) : (
              <CloudDownloadOutlined style={{ fontSize: 16, color: "#fff" }} />
            )}
          </div>
          <div>
            <Text style={{ fontSize: 16, fontWeight: 700, display: "block", color: "#111827" }}>
              {isCurrent ? "当前版本更新日志" : "发现新版本"}
            </Text>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}>
              <Tag
                icon={<TagOutlined />}
                color={isCurrent ? "blue" : "green"}
                style={{ margin: 0, fontWeight: 600 }}
              >
                v{version}
              </Tag>
              {!isCurrent && (
                <Tag color="default" style={{ margin: 0 }}>
                  当前 v{currentVersion}
                </Tag>
              )}
              {publishDate && (
                <Text type="secondary" style={{ fontSize: 11 }}>
                  <ClockCircleOutlined style={{ marginRight: 3 }} />
                  {publishDate}
                </Text>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Content */}
      <div style={{ padding: "16px 24px 24px" }}>
        {html === null ? (
          <Skeleton active paragraph={{ rows: 5 }} title={false} />
        ) : (
          <>
            {trimmed && (
              <Divider
                orientation="left"
                orientationMargin={0}
                style={{ margin: "0 0 12px", fontSize: 12, color: "#6b7280" }}
              >
                更新内容
              </Divider>
            )}
            <div
              className="update-notes"
              dangerouslySetInnerHTML={{ __html: html }}
            />
          </>
        )}
        {!trimmed && html !== null && (
          <Text type="secondary" style={{ fontSize: 12 }}>
            此版本发行说明为空。
          </Text>
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
