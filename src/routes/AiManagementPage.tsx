import { useState } from "react";
import {
  CloudServerOutlined,
  MessageOutlined,
  RobotOutlined,
  SafetyCertificateOutlined,
} from "@ant-design/icons";
import McpManager from "@/components/AiManagement/McpManager";
import SessionManager from "@/components/AiManagement/SessionManager";
import "@/components/AiManagement/AiManagement.css";

type AiWorkspaceView = "mcp" | "sessions";

const VIEWS: ReadonlyArray<{
  key: AiWorkspaceView;
  title: string;
  icon: React.ReactNode;
}> = [
  { key: "mcp", title: "MCP 控制台", icon: <CloudServerOutlined /> },
  { key: "sessions", title: "会话资料库", icon: <MessageOutlined /> },
];

export default function AiManagementPage() {
  const [activeView, setActiveView] = useState<AiWorkspaceView>("mcp");
  const [mountedViews, setMountedViews] = useState<ReadonlySet<AiWorkspaceView>>(
    () => new Set<AiWorkspaceView>(["mcp"]),
  );

  const activateView = (view: AiWorkspaceView) => {
    setActiveView(view);
    setMountedViews((current) => {
      if (current.has(view)) return current;
      const next = new Set(current);
      next.add(view);
      return next;
    });
  };

  return (
    <main className="ai-management-page">
      <header className="aim-top">
        <div className="aim-mark"><RobotOutlined /></div>
        <div className="aim-title">
          <h1>AI 管理中心</h1>
          <p>本机 AI 客户端的 MCP 能力与历史会话，统一编排</p>
        </div>

        <nav className="aim-tabs" data-active={activeView} aria-label="AI 管理功能">
          <span className="aim-tabs-thumb" aria-hidden="true" />
          {VIEWS.map((view) => (
            <button
              key={view.key}
              type="button"
              className={activeView === view.key ? "is-active" : ""}
              aria-current={activeView === view.key ? "page" : undefined}
              onClick={() => activateView(view.key)}
            >
              {view.icon}
              {view.title}
            </button>
          ))}
        </nav>

        <span className="aim-trust" title="所有配置与会话都在本机处理，不上传云端">
          <SafetyCertificateOutlined />
          全本地
        </span>
      </header>

      <section className="aim-workbench" aria-live="polite">
        {mountedViews.has("mcp") && (
          <div className="aim-view" hidden={activeView !== "mcp"}>
            <McpManager />
          </div>
        )}
        {mountedViews.has("sessions") && (
          <div className="aim-view" hidden={activeView !== "sessions"}>
            <SessionManager />
          </div>
        )}
      </section>
    </main>
  );
}
