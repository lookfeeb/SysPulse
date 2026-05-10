import { useEffect } from "react";
import {
  createMemoryRouter,
  RouterProvider,
  Navigate,
} from "react-router-dom";
import { ConfigProvider, App as AntdApp } from "antd";
import zhCN from "antd/locale/zh_CN";

import AppLayout from "@/components/Layout";
import { PAGES, DEFAULT_PATH } from "@/routes/registry";

import { useConfigStore, bindConfigEvents } from "@/stores/configStore";
import { bindLiveEvents, useLiveStore } from "@/stores/liveStore";
import { bindHwEvents, useHwStore } from "@/stores/hwStore";

import "@/i18n";

const router = createMemoryRouter([
  {
    path: "/",
    element: <AppLayout />,
    children: [
      { index: true, element: <Navigate to={DEFAULT_PATH} replace /> },
      ...PAGES.map((p) => ({ path: p.path, element: p.element })),
    ],
  },
], {
  future: {
    v7_relativeSplatPath: true,
  },
});

export default function App() {
  const load = useConfigStore((s) => s.load);
  const prime = useLiveStore((s) => s.prime);
  const primeHw = useHwStore((s) => s.prime);

  useEffect(() => {
    void load();
    void prime();
    void primeHw();
    void bindConfigEvents();
    void bindLiveEvents();
    void bindHwEvents();
  }, [load, prime, primeHw]);

  return (
    <ConfigProvider
      locale={zhCN}
      theme={{
        token: {
          colorPrimary: "#3388cc",
          borderRadius: 8,
          colorBgContainer: "#ffffff",
          colorBgLayout: "#f0f2f5",
          fontFamily: "'Inter', 'Microsoft YaHei UI', system-ui, sans-serif",
          fontSize: 13,
        },
        components: {
          Menu: {
            itemBorderRadius: 8,
            itemMarginInline: 8,
          },
          Card: {
            borderRadiusLG: 10,
          },
          Collapse: {
            borderRadiusLG: 8,
          },
        },
      }}
    >
      <AntdApp>
        <RouterProvider router={router} future={{ v7_startTransition: true }} />
      </AntdApp>
    </ConfigProvider>
  );
}
