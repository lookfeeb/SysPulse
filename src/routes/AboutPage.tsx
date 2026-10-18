import { useEffect, useState } from "react";
import { App as AntdApp, Button, Card, Progress, Space, Tag, Typography } from "antd";
import {
  CloudDownloadOutlined,
  CheckCircleOutlined,
  SyncOutlined,
  FolderOpenOutlined,
  FileTextOutlined,
  DatabaseOutlined,
  InfoCircleOutlined,
  WindowsOutlined,
  CodeOutlined,
  FileTextFilled,
} from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { getAppInfo, openPath, quitApp } from "@/ipc";
import type { AppInfo } from "@/bindings";
import { useUpdateStore, type UpdateStatus } from "@/stores/updateStore";
import { fmtBytes } from "@/utils/format";
import "./AboutPage.css";
import UpdateNotesModal from "@/components/UpdateNotesModal";

const { Text } = Typography;

const GITHUB_REPO = "lookfeeb/SysPulse";

/** Fetch release notes for a given version tag from GitHub API. */
async function fetchReleaseNotes(version: string): Promise<{ body: string; date: string }> {
  const tag = version.startsWith("v") ? version : `v${version}`;
  try {
    const res = await fetch(
      `https://api.github.com/repos/${GITHUB_REPO}/releases/tags/${tag}`,
      { headers: { Accept: "application/vnd.github+json" } },
    );
    if (!res.ok) return { body: "", date: "" };
    const data = (await res.json()) as { body?: string; published_at?: string };
    const date = data.published_at
      ? new Date(data.published_at).toLocaleDateString("zh-CN", {
          year: "numeric",
          month: "2-digit",
          day: "2-digit",
        })
      : "";
    return { body: data.body ?? "", date };
  } catch {
    return { body: "", date: "" };
  }
}

export default function AboutPage() {
  const { t } = useTranslation();
  const { message } = AntdApp.useApp();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [notesOpen, setNotesOpen] = useState(false);
  const [notesVersion, setNotesVersion] = useState("");
  const [notesContent, setNotesContent] = useState("");
  const [notesDate, setNotesDate] = useState("");
  const [notesLoading, setNotesLoading] = useState(false);

  const pending = useUpdateStore((s) => s.pending);
  const status = useUpdateStore((s) => s.status);
  const checkForUpdate = useUpdateStore((s) => s.checkForUpdate);
  const downloadAndInstall = useUpdateStore((s) => s.downloadAndInstall);

  useEffect(() => {
    void getAppInfo().then(setInfo);
  }, []);

  // Surface install-ready transitions as a toast.
  useEffect(() => {
    if (status.kind === "ready") {
      void message.success("更新安装完成");
    }
  }, [status.kind, message]);

  /** Open the release notes modal for a specific version. */
  const showNotes = async (version: string) => {
    setNotesVersion(version);
    setNotesContent("");
    setNotesDate("");
    setNotesLoading(true);
    setNotesOpen(true);
    const { body, date } = await fetchReleaseNotes(version);
    // If GitHub API returned minimal content, fallback to updater notes.
    let content = body;
    if (!content || content.length < 20) {
      if (status.kind === "available" && status.version === version && status.notes) {
        content = status.notes;
      }
    }
    setNotesContent(content);
    setNotesDate(date);
    setNotesLoading(false);
  };

  if (!info) return null;

  const isChecking = status.kind === "checking";
  const isDownloading = status.kind === "downloading" || status.kind === "verifying" || status.kind === "installing";

  return (
    <Space vertical size={16} style={{ width: "100%" }}>
      {/* App Identity */}
      <Card style={{ borderRadius: 10 }} styles={{ body: { padding: "20px 24px" } }}>
        <div className="about-app-identity">
          <div
            style={{
              width: 48,
              height: 48,
              borderRadius: 12,
              background: "linear-gradient(135deg, #3388cc, #1d4ed8)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              flexShrink: 0,
            }}
          >
            <InfoCircleOutlined style={{ fontSize: 22, color: "#fff" }} />
          </div>
          <div style={{ flex: 1 }}>
            <Text style={{ fontSize: 18, fontWeight: 700, display: "block" }}>{info.name}</Text>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4, flexWrap: "wrap" }}>
              <Tag
                color="blue"
                style={{ margin: 0, fontWeight: 600, cursor: "pointer" }}
                onClick={() => void showNotes(info.version)}
                title="点击查看当前版本更新日志"
              >
                v{info.version}
              </Tag>
              <Text type="secondary" style={{ fontSize: 12 }}>
                <WindowsOutlined style={{ marginRight: 4 }} />
                {info.os} / {info.arch}
              </Text>
              <UpdateBadge status={status} />
            </div>
          </div>
          <div className="about-update-actions">
            <Button
              icon={<SyncOutlined spin={isChecking} />}
              loading={isChecking}
              onClick={() => void checkForUpdate()}
              disabled={isDownloading}
              size="small"
            >
              检查更新
            </Button>
            {status.kind === "available" && (
              <Button
                type="primary"
                icon={<CloudDownloadOutlined />}
                onClick={() => void downloadAndInstall()}
                size="small"
                style={{ background: "#22c55e", borderColor: "#22c55e" }}
              >
                安装 v{status.version}
              </Button>
            )}
            {status.kind === "error" && pending && (
              <Button type="primary" size="small" icon={<CloudDownloadOutlined />} onClick={() => void downloadAndInstall()}>
                {status.operation === "install" ? "重试下载" : `安装 v${pending.version}`}
              </Button>
            )}
            {status.kind === "ready" && (
              <Button
                type="primary"
                size="small"
                onClick={() => void quitApp()}
                style={{ background: "#22c55e", borderColor: "#22c55e" }}
              >
                关闭应用
              </Button>
            )}
          </div>
        </div>

        {status.kind === "downloading" && (
          <div className="about-download" aria-live="polite">
            <div className="about-download-heading"><strong>{status.downloaded ? "正在下载更新" : "正在连接下载服务"}</strong><span>{status.bytesPerSecond > 0 ? `${fmtBytes(status.bytesPerSecond)}/s` : "连接中…"}</span></div>
            <Progress percent={status.progress ?? 0} showInfo={status.progress !== null} size="small" strokeColor="#3388cc" className={status.progress === null ? "is-indeterminate" : ""} />
            <div className="about-download-meta"><span>{fmtBytes(status.downloaded)}{status.total ? ` / ${fmtBytes(status.total)}` : " · 正在获取文件大小"}</span><span>可切换页面，下载会继续</span></div>
          </div>
        )}
        {(status.kind === "verifying" || status.kind === "installing") && (
          <div className="about-download"><SyncOutlined spin /> {status.kind === "verifying" ? "正在校验更新包签名…" : "正在启动安装程序…"}</div>
        )}
        {status.kind === "available" && (
          <div style={{ marginTop: 10 }}>
            <Button
              type="link"
              size="small"
              icon={<FileTextFilled />}
              onClick={() => void showNotes(status.version)}
              style={{ padding: 0, height: "auto", fontSize: 12 }}
            >
              查看 v{status.version} 更新说明
            </Button>
          </div>
        )}
        {status.kind === "error" && (
          <Text type="danger" style={{ display: "block", fontSize: 12, marginTop: 10 }}>
            {status.operation === "install" ? "更新失败" : "检查失败"}：{status.message}
          </Text>
        )}
      </Card>

      <UpdateNotesModal
        open={notesOpen}
        version={notesVersion}
        currentVersion={info.version}
        notes={notesContent}
        loading={notesLoading}
        publishDate={notesDate}
        onClose={() => setNotesOpen(false)}
      />

      {/* Paths */}
      <Card
        size="small"
        title={
          <span style={{ fontSize: 13, fontWeight: 600, color: "#374151" }}>
            <FolderOpenOutlined style={{ marginRight: 8, color: "#3388cc" }} />
            路径信息
          </span>
        }
        style={{ borderRadius: 10 }}
        styles={{ body: { padding: "12px 16px" } }}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <PathRow
            icon={<FolderOpenOutlined />}
            label="配置目录"
            path={info.configDir}
            onOpen={() => void openPath(info.configDir)}
          />
          <PathRow
            icon={<FileTextOutlined />}
            label="日志目录"
            path={info.logsDir}
            onOpen={() => void openPath(info.logsDir)}
          />
          <PathRow
            icon={<DatabaseOutlined />}
            label="数据库"
            path={info.dbFile}
          />
        </div>
      </Card>

      {/* Attribution */}
      <Card
        size="small"
        title={
          <span style={{ fontSize: 13, fontWeight: 600, color: "#374151" }}>
            <CodeOutlined style={{ marginRight: 8, color: "#3388cc" }} />
            {t("about.attribution")}
          </span>
        }
        style={{ borderRadius: 10 }}
        styles={{ body: { padding: "12px 16px" } }}
      >
        <Text type="secondary" style={{ fontSize: 12, lineHeight: 1.6 }}>
          {t("about.lhmAttribution")}
        </Text>
      </Card>
    </Space>
  );
}

function PathRow({
  icon,
  label,
  path,
  onOpen,
}: {
  icon: React.ReactNode;
  label: string;
  path: string;
  onOpen?: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "6px 10px",
        background: "#f9fafb",
        borderRadius: 6,
        border: "1px solid #f0f0f0",
      }}
    >
      <span style={{ color: "#6b7280", fontSize: 13, flexShrink: 0 }}>{icon}</span>
      <Text type="secondary" style={{ fontSize: 11, flexShrink: 0, width: 56 }}>{label}</Text>
      <Text copyable style={{ fontSize: 11, flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {path}
      </Text>
      {onOpen && (
        <Button type="text" size="small" onClick={onOpen} style={{ fontSize: 11, color: "#3388cc", padding: "0 6px" }}>
          打开
        </Button>
      )}
    </div>
  );
}

function UpdateBadge({ status }: { status: UpdateStatus }) {
  switch (status.kind) {
    case "checking":
      return <Tag icon={<SyncOutlined spin />} color="processing" style={{ margin: 0 }}>检查中</Tag>;
    case "latest":
      return <Tag icon={<CheckCircleOutlined />} color="success" style={{ margin: 0 }}>已是最新</Tag>;
    case "available":
      return <Tag icon={<CloudDownloadOutlined />} color="warning" style={{ margin: 0 }}>有新版本</Tag>;
    case "downloading":
      return <Tag icon={<SyncOutlined spin />} color="processing" style={{ margin: 0 }}>下载中{status.progress === null ? "" : ` ${status.progress}%`}</Tag>;
    case "verifying":
      return <Tag icon={<SyncOutlined spin />} color="processing" style={{ margin: 0 }}>校验中</Tag>;
    case "installing":
      return <Tag icon={<SyncOutlined spin />} color="processing" style={{ margin: 0 }}>安装中</Tag>;
    case "ready":
      return <Tag icon={<CheckCircleOutlined />} color="success" style={{ margin: 0 }}>更新就绪</Tag>;
    case "error":
      return <Tag color="error" style={{ margin: 0 }}>{status.operation === "install" ? "更新失败" : "检查失败"}</Tag>;
    default:
      return null;
  }
}
