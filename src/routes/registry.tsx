import type { ReactNode } from "react";
import {
  AppstoreOutlined,
  FireOutlined,
  HistoryOutlined,
  InfoCircleOutlined,
  SettingOutlined,
} from "@ant-design/icons";

import DashboardPage from "@/routes/DashboardPage";
import GeneralPage from "@/routes/GeneralPage";
import HardwarePage from "@/routes/HardwarePage";
import HistoryPage from "@/routes/HistoryPage";
import AboutPage from "@/routes/AboutPage";

export interface PageDef {
  path: string;
  labelKey: string;
  icon: ReactNode;
  element: ReactNode;
}

export const PAGES: PageDef[] = [
  { path: "dashboard", labelKey: "menu.dashboard", icon: <AppstoreOutlined />, element: <DashboardPage /> },
  { path: "general", labelKey: "menu.general", icon: <SettingOutlined />, element: <GeneralPage /> },
  { path: "hardware", labelKey: "menu.hardware", icon: <FireOutlined />, element: <HardwarePage /> },
  { path: "history", labelKey: "menu.history", icon: <HistoryOutlined />, element: <HistoryPage /> },
  { path: "about", labelKey: "menu.about", icon: <InfoCircleOutlined />, element: <AboutPage /> },
];

export const DEFAULT_PATH = `/${PAGES[0].path}`;
