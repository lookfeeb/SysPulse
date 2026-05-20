import { Layout, Menu, Tooltip } from "antd";
import { useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { PAGES, DEFAULT_PATH } from "@/routes/registry";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getAppInfo } from "@/ipc";

const { Sider } = Layout;

const PROJECT_URL = "https://github.com/lookfeeb/SysPulse";

export default function SideMenu() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const [version, setVersion] = useState<string | null>(null);
  const selectedKey =
    location.pathname === "/" ? DEFAULT_PATH : location.pathname;

  useEffect(() => {
    void getAppInfo()
      .then((info) => setVersion(info.version))
      .catch(() => setVersion(null));
  }, []);

  return (
    <Sider
      width={176}
      theme="light"
      style={{
        borderRight: "1px solid #e8eaed",
        background: "#fafbfc",
        boxShadow: "1px 0 0 #e8eaed",
      }}
    >
      {/* Logo 区域 */}
      <div
        style={{
          height: 56,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          borderBottom: "1px solid #e8eaed",
          marginBottom: 8,
        }}
      >
        <Tooltip title={`SysPulse${version ? ` v${version}` : ""}`} placement="right">
          <div
            onClick={() => void openUrl(PROJECT_URL)}
            style={{
              width: 32,
              height: 32,
              borderRadius: 8,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              flexShrink: 0,
              overflow: "hidden",
              cursor: "pointer",
              transition: "transform 0.15s ease",
            }}
            onMouseEnter={(e) => { e.currentTarget.style.transform = "scale(1.1)"; }}
            onMouseLeave={(e) => { e.currentTarget.style.transform = "scale(1)"; }}
          >
            <img
              src="/app-icon.png"
              alt="SysPulse"
              style={{ width: 32, height: 32, display: "block" }}
            />
          </div>
        </Tooltip>
      </div>

      <Menu
        mode="inline"
        selectedKeys={[selectedKey]}
        onClick={({ key }) => navigate(key)}
        style={{ background: "transparent", border: "none" }}
        items={PAGES.map((p) => ({
          key: `/${p.path}`,
          icon: p.icon,
          label: t(p.labelKey),
        }))}
      />
    </Sider>
  );
}
