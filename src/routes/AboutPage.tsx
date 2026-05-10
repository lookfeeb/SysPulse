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
} from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { getAppInfo, openPath, quitApp } from "@/ipc";
import type { AppInfo } from "@/bindings";
import { check } from "@tauri-apps/plugin-updater";

const { Text } = Typography;

type UpdateState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "latest" }
  | { status: "available"; version: string; notes: string }
  | { status: "downloading"; progress: number }
  | { status: "ready" }
  | { status: "error"; message: string };

export default function AboutPage() {
  const { t } = useTranslation();
  const { message } = AntdApp.useApp();
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [updateState, setUpdateState] = useState<UpdateState>({ status: "idle" });

  useEffect(() => {
    void getAppInfo().then(setInfo);
  }, []);

  const checkForUpdate = async () => {
    setUpdateState({ status: "checking" });
    try {
      const update = await check();
      if (update) {
        setUpdateState({
          status: "available",
          version: update.version,
          notes: update.body ?? "",
        });
      } else {
        setUpdateState({ status: "latest" });
      }
    } catch (e: unknown) {
      setUpdateState({
        status: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  };

  const doUpdate = async () => {
    setUpdateState({ status: "downloading", progress: 0 });
    try {
      const update = await check();
      if (!update) {
        setUpdateState({ status: "latest" });
        return;
      }
      let downloaded = 0;
      let contentLength = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength) {
          contentLength = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          const pct = contentLength > 0 ? Math.round((downloaded / contentLength) * 100) : 0;
          setUpdateState({ status: "downloading", progress: pct });
        } else if (event.event === "Finished") {
          setUpdateState({ status: "ready" });
        }
      });
      setUpdateState({ status: "ready" });
      void message.success("更新已下载，重启后生效");
    } catch (e: unknown) {
      setUpdateState({
        status: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  };

  if (!info) return null;

  return (
    <Space direction="vertical" size={16} style={{ width: "100%" }}>
      {/* App Identity */}
      <Card style={{ borderRadius: 10 }} styles={{ body: { padding: "20px 24px" } }}>
        <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
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
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}>
              <Tag color="blue" style={{ margin: 0, fontWeight: 600 }}>v{info.version}</Tag>
              <Text type="secondary" style={{ fontSize: 12 }}>
                <WindowsOutlined style={{ marginRight: 4 }} />
                {info.os} / {info.arch}
              </Text>
              <UpdateBadge state={updateState} />
            </div>
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <Button
              icon={<SyncOutlined spin={updateState.status === "checking"} />}
              loading={updateState.status === "checking"}
              onClick={() => void checkForUpdate()}
              disabled={updateState.status === "downloading"}
              size="small"
            >
              检查更新
            </Button>
            {updateState.status === "available" && (
              <Button
                type="primary"
                icon={<CloudDownloadOutlined />}
                onClick={() => void doUpdate()}
                size="small"
                style={{ background: "#22c55e", borderColor: "#22c55e" }}
              >
                安装 v{updateState.version}
              </Button>
            )}
            {updateState.status === "ready" && (
              <Button
                type="primary"
                size="small"
                onClick={() => void quitApp()}
                style={{ background: "#22c55e", borderColor: "#22c55e" }}
              >
                重启应用
              </Button>
            )}
          </div>
        </div>

        {updateState.status === "downloading" && (
          <Progress
            percent={updateState.progress}
            size="small"
            strokeColor="#3388cc"
            style={{ maxWidth: 320, marginTop: 12 }}
          />
        )}
        {updateState.status === "available" && updateState.notes && (
          <Text type="secondary" style={{ display: "block", fontSize: 12, marginTop: 10 }}>
            更新说明：{updateState.notes}
          </Text>
        )}
        {updateState.status === "error" && (
          <Text type="danger" style={{ display: "block", fontSize: 12, marginTop: 10 }}>
            检查失败：{updateState.message}
          </Text>
        )}
      </Card>

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

function UpdateBadge({ state }: { state: UpdateState }) {
  switch (state.status) {
    case "latest":
      return <Tag icon={<CheckCircleOutlined />} color="success" style={{ margin: 0 }}>已是最新</Tag>;
    case "available":
      return <Tag icon={<CloudDownloadOutlined />} color="warning" style={{ margin: 0 }}>有新版本</Tag>;
    case "downloading":
      return <Tag icon={<SyncOutlined spin />} color="processing" style={{ margin: 0 }}>下载中 {state.progress}%</Tag>;
    case "ready":
      return <Tag icon={<CheckCircleOutlined />} color="success" style={{ margin: 0 }}>更新就绪</Tag>;
    case "error":
      return <Tag color="error" style={{ margin: 0 }}>检查失败</Tag>;
    default:
      return null;
  }
}
