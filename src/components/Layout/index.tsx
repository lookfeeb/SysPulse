import { Suspense, useEffect } from "react";
import { App, Layout, Spin } from "antd";
import { Outlet } from "react-router-dom";
import { useCleanupStore } from "@/stores/cleanupStore";
import SideMenu from "./SideMenu";
import AdminBanner from "./AdminBanner";

const { Content } = Layout;

export default function AppLayout() {
  const notice = useCleanupStore((state) => state.notice);
  const { message } = App.useApp();
  useEffect(() => {
    if (notice) void message[notice.type]({ content: notice.text, key: "cleanup-task", duration: 4 });
  }, [message, notice]);

  return (
    <Layout style={{ height: "100vh", background: "#f0f2f5" }}>
      <SideMenu />
      <Layout style={{ background: "#f0f2f5" }}>
        <Content
          className="app-content-scroll"
          style={{
            overflow: "auto",
            padding: "20px 24px",
            background: "#f0f2f5",
          }}
        >
          <AdminBanner />
          <Suspense fallback={<div role="status" aria-label="正在加载页面"><Spin /></div>}>
            <Outlet />
          </Suspense>
        </Content>
      </Layout>
    </Layout>
  );
}
