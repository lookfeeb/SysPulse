import { lazy, type ReactNode } from "react";
import {
  AppstoreOutlined,
  ClearOutlined,
  FireOutlined,
  HistoryOutlined,
  InfoCircleOutlined,
  RobotOutlined,
  SettingOutlined,
} from "@ant-design/icons";

const DashboardPage = lazy(() => import("@/routes/DashboardPage"));
const GeneralPage = lazy(() => import("@/routes/GeneralPage"));
const HardwarePage = lazy(() => import("@/routes/HardwarePage"));
const HistoryPage = lazy(() => import("@/routes/HistoryPage"));
const CleanupPage = lazy(() => import("@/routes/CleanupPage"));
const AiManagementPage = lazy(() => import("@/routes/AiManagementPage"));
const AboutPage = lazy(() => import("@/routes/AboutPage"));

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
  { path: "cleanup", labelKey: "menu.cleanup", icon: <ClearOutlined />, element: <CleanupPage /> },
  { path: "ai-management", labelKey: "menu.aiManagement", icon: <RobotOutlined />, element: <AiManagementPage /> },
  { path: "about", labelKey: "menu.about", icon: <InfoCircleOutlined />, element: <AboutPage /> },
];

export const DEFAULT_PATH = `/${PAGES[0].path}`;
