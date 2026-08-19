import { useState } from "react";
import {
  ApiOutlined,
  CloudServerOutlined,
  MessageOutlined,
  RobotOutlined,
  SafetyCertificateOutlined,
} from "@ant-design/icons";
import McpManager from "@/components/AiManagement/McpManager";
import SessionManager from "@/components/AiManagement/SessionManager";

type AiWorkspaceView = "mcp" | "sessions";

const VIEWS: ReadonlyArray<{
  key: AiWorkspaceView;
  title: string;
  description: string;
  icon: React.ReactNode;
}> = [
  {
    key: "mcp",
    title: "MCP 控制台",
    description: "管理客户端工具与授权",
    icon: <CloudServerOutlined />,
  },
  {
    key: "sessions",
    title: "会话资料库",
    description: "检索、阅读与导出记录",
    icon: <MessageOutlined />,
  },
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
      <header className="ai-command-header">
        <div className="ai-command-glow" aria-hidden="true" />
        <div className="ai-command-intro">
          <div className="ai-command-mark"><RobotOutlined /></div>
          <div>
            <span className="ai-command-eyebrow">LOCAL AI OPERATIONS</span>
            <h1>AI 管理中心</h1>
            <p>集中编排本机 AI 客户端的 MCP 能力，并管理跨工具历史会话。</p>
          </div>
        </div>

        <div className="ai-command-status" aria-label="AI 管理能力概览">
          <div>
            <ApiOutlined />
            <span><b>3</b> 个 MCP 客户端</span>
          </div>
          <div>
            <SafetyCertificateOutlined />
            <span><b>本地</b> 数据处理</span>
          </div>
        </div>

        <nav className="ai-view-switcher" aria-label="AI 管理功能">
          {VIEWS.map((view) => (
            <button
              key={view.key}
              type="button"
              className={activeView === view.key ? "is-active" : ""}
              aria-current={activeView === view.key ? "page" : undefined}
              onClick={() => activateView(view.key)}
            >
              <span className="ai-view-switcher-icon">{view.icon}</span>
              <span>
                <strong>{view.title}</strong>
                <small>{view.description}</small>
              </span>
            </button>
          ))}
        </nav>
      </header>

      <section className="ai-workbench" aria-live="polite">
        {mountedViews.has("mcp") && (
          <div className="ai-workbench-view" hidden={activeView !== "mcp"}>
            <McpManager />
          </div>
        )}
        {mountedViews.has("sessions") && (
          <div className="ai-workbench-view" hidden={activeView !== "sessions"}>
            <SessionManager />
          </div>
        )}
      </section>
    </main>
  );
}
