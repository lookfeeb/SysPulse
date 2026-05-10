import { useEffect, useState } from "react";
import {
  Button,
  Card,
  DatePicker,
  App as AntdApp,
  Radio,
  Space,
  Table,
  Typography,
} from "antd";
import dayjs, { type Dayjs } from "dayjs";
import { useTranslation } from "react-i18next";
import { queryTrafficHistory } from "@/ipc";
import type { DailyTraffic, HistoryQuery } from "@/bindings";
import { save } from "@tauri-apps/plugin-dialog";
import { exportTrafficCsv } from "@/ipc";
import { fmtBytes } from "@/utils/format";

const { RangePicker } = DatePicker;

export default function HistoryPage() {
  const { t } = useTranslation();
  const { message } = AntdApp.useApp();
  const [range, setRange] = useState<[Dayjs, Dayjs]>([
    dayjs().subtract(30, "day"),
    dayjs(),
  ]);
  const [granularity, setGranularity] = useState<"day" | "month">("day");
  const [rows, setRows] = useState<DailyTraffic[]>([]);
  const [loading, setLoading] = useState(false);

  const buildQuery = (): HistoryQuery => ({
    from: range[0].format("YYYY-MM-DD"),
    to: range[1].format("YYYY-MM-DD"),
    granularity,
    iface: null,
  });

  const onQuery = async () => {
    setLoading(true);
    try {
      const r = await queryTrafficHistory(buildQuery());
      setRows([...r].reverse());
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void onQuery();
  }, [range, granularity]);

  const onExport = async () => {
    const path = await save({
      defaultPath: `traffic-${dayjs().format("YYYYMMDD")}.csv`,
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!path) return;
    try {
      const r = await exportTrafficCsv(buildQuery(), path);
      void message.success(`已导出 ${r.rows} 行 → ${r.savedTo}`);
    } catch (e: unknown) {
      void message.error(e instanceof Error ? e.message : String(e));
    }
  };

  const columns = [
    { title: "日期/月份", dataIndex: "date", width: 140 },
    {
      title: "下行",
      dataIndex: "bytesRecv",
      render: (v: number) => fmtBytes(v),
    },
    {
      title: "上行",
      dataIndex: "bytesSent",
      render: (v: number) => fmtBytes(v),
    },
    {
      title: "合计",
      render: (_: unknown, r: DailyTraffic) =>
        fmtBytes(r.bytesRecv + r.bytesSent),
    },
  ];

  return (
    <Card title={t("menu.history")}>
      <Space style={{ marginBottom: 16 }}>
        <RangePicker
          value={range}
          onChange={(v) => v && setRange(v as [Dayjs, Dayjs])}
        />
        <Radio.Group
          value={granularity}
          onChange={(e) => setGranularity(e.target.value)}
        >
          <Radio.Button value="day">{t("history.day")}</Radio.Button>
          <Radio.Button value="month">{t("history.month")}</Radio.Button>
        </Radio.Group>
        <Button type="primary" loading={loading} onClick={onQuery}>
          {t("history.query")}
        </Button>
        <Button onClick={onExport} disabled={rows.length === 0}>
          {t("history.exportCsv")}
        </Button>
      </Space>

      {rows.length === 0 && !loading ? (
        <Typography.Text type="secondary">暂无流量历史数据。</Typography.Text>
      ) : (
        <Table
          size="small"
          rowKey="date"
          loading={loading}
          pagination={{ pageSize: 10, showSizeChanger: false, showTotal: (total) => `共 ${total} 条` }}
          dataSource={rows}
          columns={columns}
        />
      )}
    </Card>
  );
}
