import { useEffect, useState } from "react";
import { App as AntdApp, Card, Form, InputNumber, Select, Space, Switch, Typography } from "antd";
import { ThunderboltOutlined, DashboardOutlined, PoweroffOutlined, GlobalOutlined } from "@ant-design/icons";
import { useTranslation } from "react-i18next";
import { useConfigStore } from "@/stores/configStore";
import { commands } from "@/bindings";
import type { GeneralConfig } from "@/bindings";
import { OverlaySettingsCard } from "@/routes/OverlayPage";

const { Text } = Typography;

export default function GeneralPage() {
  const { t, i18n } = useTranslation();
  const { message } = AntdApp.useApp();
  const config = useConfigStore((s) => s.config);
  const patch = useConfigStore((s) => s.patch);
  const [autoStart, setAutoStart] = useState(false);
  const [autoStartLoading, setAutoStartLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setAutoStartLoading(true);
    void commands
      .autostartIsEnabled()
      .then((enabled) => {
        if (!cancelled) setAutoStart(enabled);
      })
      .catch((e: unknown) => {
        if (!cancelled) void message.error(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setAutoStartLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [message]);

  if (!config) return null;

  const apply = async (changes: Partial<GeneralConfig>) => {
    try {
      await patch({ general: { ...config.general, ...changes } });
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    }
  };

  const onAutoStartChange = async (checked: boolean) => {
    setAutoStartLoading(true);
    try {
      if (checked) await commands.autostartEnable();
      else await commands.autostartDisable();
      setAutoStart(await commands.autostartIsEnabled());
    } catch (e: unknown) {
      setAutoStart(await commands.autostartIsEnabled().catch(() => autoStart));
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setAutoStartLoading(false);
    }
  };

  const adaptive = config.general.adaptiveInterval;

  return (
    <Space direction="vertical" size={16} style={{ width: "100%" }}>
      <OverlaySettingsCard />
      <Card title={t("menu.general")} style={{ borderRadius: 10 }}>
        <Form layout="horizontal" labelCol={{ span: 6 }} wrapperCol={{ span: 14 }}>
          {/* 开机自启 */}
          <Form.Item label={t("general.autoStart")} style={{ marginBottom: 20 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <PoweroffOutlined style={{ color: autoStart ? "#22c55e" : "#9ca3af", fontSize: 14 }} />
              <Switch
                checked={autoStart}
                loading={autoStartLoading}
                onChange={(checked) => void onAutoStartChange(checked)}
              />
              <Text type="secondary" style={{ fontSize: 12 }}>
                {autoStart ? "已开启" : "已关闭"}
              </Text>
            </div>
          </Form.Item>

          {/* 语言切换 */}
          <Form.Item label={t("general.language")} style={{ marginBottom: 20 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <GlobalOutlined style={{ color: "#3388cc", fontSize: 14 }} />
              <Select
                value={i18n.language}
                onChange={(value) => {
                  void i18n.changeLanguage(value);
                  localStorage.setItem("lang", value);
                }}
                style={{ width: 160 }}
                size="small"
                options={[
                  { value: "zh-CN", label: "简体中文" },
                  { value: "en-US", label: "English" },
                ]}
              />
            </div>
          </Form.Item>

          {/* 采样间隔（合并自适应开关） */}
          <Form.Item label={t("general.sampleInterval")} style={{ marginBottom: 0 }}>
            <div
              style={{
                background: "#f9fafb",
                borderRadius: 8,
                padding: "12px 14px",
                border: "1px solid #f0f0f0",
              }}
            >
              {/* 自适应开关行 */}
              <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: adaptive ? 6 : 10 }}>
                <ThunderboltOutlined style={{ color: adaptive ? "#3388cc" : "#9ca3af", fontSize: 14 }} />
                <Switch
                  size="small"
                  checked={adaptive}
                  onChange={(checked) => void apply({ adaptiveInterval: checked })}
                />
                <Text style={{ fontSize: 12, fontWeight: 500 }}>
                  自适应采样
                </Text>
                {adaptive && (
                  <Text
                    style={{
                      fontSize: 11,
                      color: "#3388cc",
                      background: "#eff6ff",
                      padding: "1px 8px",
                      borderRadius: 10,
                      fontWeight: 500,
                    }}
                  >
                    500~2000ms
                  </Text>
                )}
              </div>

              {adaptive ? (
                <Text type="secondary" style={{ fontSize: 11, display: "block", paddingLeft: 24 }}>
                  多信号融合算法，根据 CPU / 网络活跃度及变化率自动调整采样频率。并行采集、连接复用，启动即出数据。
                </Text>
              ) : (
                <>
                  {/* 手动间隔输入 */}
                  <div style={{ display: "flex", alignItems: "center", gap: 10, paddingLeft: 24 }}>
                    <DashboardOutlined style={{ color: "#6b7280", fontSize: 13 }} />
                    <InputNumber
                      min={500}
                      max={5000}
                      step={250}
                      value={config.general.sampleIntervalMs}
                      addonAfter="ms"
                      size="small"
                      style={{ width: 150 }}
                      onChange={(value) => {
                        if (typeof value === "number") void apply({ sampleIntervalMs: value });
                      }}
                    />
                    <Text type="secondary" style={{ fontSize: 11 }}>
                      {config.general.sampleIntervalMs <= 500
                        ? "最高精度"
                        : config.general.sampleIntervalMs >= 3000
                          ? "省电模式"
                          : "均衡"}
                    </Text>
                  </div>
                  <Text type="secondary" style={{ fontSize: 11, display: "block", marginTop: 6, paddingLeft: 24 }}>
                    {t("general.sampleIntervalHint")}
                  </Text>
                </>
              )}
            </div>
          </Form.Item>
        </Form>
      </Card>
    </Space>
  );
}
