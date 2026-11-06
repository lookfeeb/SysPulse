import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Alert,
  Card,
  DatePicker,
  App as AntdApp,
  Radio,
  Space,
  Statistic,
  Table,
  Typography,
} from "antd";
import dayjs from "dayjs";
import "dayjs/locale/zh-cn";
import zhCN from "antd/locale/zh_CN";
import { useTranslation } from "react-i18next";
import type { DailyTraffic, HistoryQuery } from "@/bindings";
import { save } from "@tauri-apps/plugin-dialog";
import { exportTrafficCsv } from "@/ipc";
import { fmtBytes } from "@/utils/format";
import { trafficTotals, useHistoryStore } from "@/stores/historyStore";
import { historyQueryFromRange, historyRangeLabel, type HistoryRange } from "@/utils/historyQuery";
import "./HistoryPage.css";

const { RangePicker } = DatePicker;
const dateLocale = {
  ...zhCN.DatePicker!,
  lang: { ...zhCN.DatePicker!.lang, monthFormat: "M月" },
};

function defaultQuery(): HistoryQuery {
  return {
    from: dayjs().subtract(30, "day").format("YYYY-MM-DD"),
    to: dayjs().format("YYYY-MM-DD"),
    granularity: "day",
    iface: null,
  };
}

export default function HistoryPage() {
  const { t } = useTranslation();
  const { message } = AntdApp.useApp();
  const { query, rows, loading, error, updatedAt, load } = useHistoryStore();
  const [initialQuery] = useState(() => query ?? defaultQuery());
  const [range, setRange] = useState<HistoryRange>([
    initialQuery.from ? dayjs(initialQuery.from) : null,
    initialQuery.to ? dayjs(initialQuery.to) : null,
  ]);
  const [granularity, setGranularity] = useState<"day" | "month">(
    initialQuery.granularity ?? "day",
  );
  const [page, setPage] = useState(1);
  const totals = useMemo(() => trafficTotals(rows), [rows]);

  const onQuery = async () => {
    if (await load(historyQueryFromRange(range, granularity))) setPage(1);
  };

  useEffect(() => {
    const refresh = () => {
      if (document.hidden) return;
      const state = useHistoryStore.getState();
      if (state.loading || state.refreshing) return;
      if (state.query) void state.refresh();
      else void state.load(initialQuery, state.updatedAt !== null);
    };
    refresh();
    const timer = window.setInterval(refresh, 1000);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [initialQuery]);

  const onExport = async () => {
    if (!query) return;
    try {
      const path = await save({
        defaultPath: `traffic-${query.granularity}-${dayjs().format("YYYYMMDD")}.csv`,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) return;
      const r = await exportTrafficCsv(query, path);
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
    <Card title={t("menu.history")} className="traffic-history">
      <Space wrap style={{ marginBottom: 16 }}>
        <RangePicker
          locale={dateLocale}
          allowEmpty={[true, true]}
          picker={granularity === "month" ? "month" : "date"}
          format={granularity === "month" ? "YYYY-MM" : "YYYY-MM-DD"}
          value={range}
          onChange={(value) => setRange(value ?? [null, null])}
        />
        <Radio.Group
          value={granularity}
          onChange={(e) => {
            const nextGranularity = e.target.value as "day" | "month";
            setGranularity(nextGranularity);
            if (nextGranularity === "month") {
              setRange(([from, to]) => [from?.startOf("month") ?? null, to?.startOf("month") ?? null]);
            }
          }}
        >
          <Radio.Button value="day">{t("history.day")}</Radio.Button>
          <Radio.Button value="month">{t("history.month")}</Radio.Button>
        </Radio.Group>
        <Button type="primary" loading={loading} onClick={onQuery} style={{ minWidth: 72 }}>
          {t("history.query")}
        </Button>
        <Button onClick={onExport} disabled={loading || rows.length === 0}>
          {t("history.exportCsv")}
        </Button>
      </Space>

      <div className="traffic-history-summary" aria-label="查询区间流量统计">
        <Statistic title="累计下行" value={totals.recv} formatter={() => updatedAt === null ? "--" : fmtBytes(totals.recv)} />
        <Statistic title="累计上行" value={totals.sent} formatter={() => updatedAt === null ? "--" : fmtBytes(totals.sent)} />
        <Statistic title="流量合计" value={totals.total} formatter={() => updatedAt === null ? "--" : fmtBytes(totals.total)} />
      </div>
      <div className="traffic-history-caption">
        <Typography.Text type="secondary">
          {query ? `${historyRangeLabel(query)} · 全部 ${rows.length} 条记录` : "正在读取流量统计…"}
        </Typography.Text>
        <Typography.Text type="secondary">
          {error ? "更新失败，保留上次数据" : updatedAt === null ? "等待数据" : `每秒更新 · ${dayjs(updatedAt).format("HH:mm:ss")}`}
        </Typography.Text>
      </div>
      {error && <Alert type="warning" showIcon title={`流量查询失败：${error}`} style={{ marginBottom: 16 }} />}

      {rows.length === 0 && !loading ? (
        <Typography.Text type="secondary">暂无流量历史数据。</Typography.Text>
      ) : (
        <Table
          size="small"
          rowKey="date"
          loading={loading}
          pagination={{ current: page, onChange: setPage, pageSize: 10, showSizeChanger: false, showTotal: (total) => `共 ${total} 条` }}
          dataSource={rows}
          columns={columns}
        />
      )}
    </Card>
  );
}
