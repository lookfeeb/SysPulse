import { Layout } from "antd";
import { Outlet } from "react-router-dom";
import SideMenu from "./SideMenu";
import AdminBanner from "./AdminBanner";

const { Content } = Layout;

export default function AppLayout() {
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
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
