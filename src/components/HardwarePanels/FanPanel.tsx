import { App as AntdApp, Radio, Slider, Typography } from "antd";
import { DashboardOutlined } from "@ant-design/icons";
import { useEffect, useMemo, useState } from "react";
import { resetFanControl, setFanManual, setFanCurve } from "@/ipc";
import { useHwStore } from "@/stores/hwStore";
import type { FanHw, FanCurvePoint } from "@/bindings";

const { Text } = Typography;

const OPTIMIZED_CURVE: FanCurvePoint[] = [
  { tempC: 0, pwm: 20 },
  { tempC: 40, pwm: 20 },
  { tempC: 50, pwm: 30 },
  { tempC: 60, pwm: 45 },
  { tempC: 70, pwm: 65 },
  { tempC: 75, pwm: 80 },
  { tempC: 80, pwm: 92 },
  { tempC: 85, pwm: 100 },
];

type FanMode = "bios" | "curve" | "manual";

function modeLabel(mode: FanMode): string {
  if (mode === "bios") return "BIOS 自动";
  if (mode === "curve") return "温控曲线";
  return "手动";
}

function modeColor(mode: FanMode): string {
  if (mode === "bios") return "#22c55e";
  if (mode === "curve") return "#8b5cf6";
  return "#f97316";
}

/** Interpolate PWM from the curve at a given temperature */
function interpolateCurve(curve: FanCurvePoint[], tempC: number): number {
  if (tempC <= curve[0].tempC) return curve[0].pwm;
  if (tempC >= curve[curve.length - 1].tempC) return curve[curve.length - 1].pwm;
  for (let i = 0; i < curve.length - 1; i++) {
    const a = curve[i];
    const b = curve[i + 1];
    if (tempC >= a.tempC && tempC <= b.tempC) {
      const t = (tempC - a.tempC) / (b.tempC - a.tempC);
      return a.pwm + t * (b.pwm - a.pwm);
    }
  }
  return curve[curve.length - 1].pwm;
}

export default function FanPanel({
  fans,
  isAdmin,
  disabled,
}: {
  fans: FanHw[];
  isAdmin: boolean;
  disabled: boolean;
}) {
  const { message } = AntdApp.useApp();
  const fanControl = useHwStore((s) => s.fanControl);
  const setFanControl = useHwStore((s) => s.setFanControl);
  const cpuTemp = useHwStore((s) => s.current?.cpu?.packageTempC ?? null);
  const [saving, setSaving] = useState(false);
  const [manualPwm, setManualPwm] = useState(40);

  const cpuFan = useMemo(() => {
    const byName = fans.find((f) => f.name.toLowerCase().includes("cpu"));
    if (byName) return byName;
    const withRpm = fans.find((f) => (f.rpm ?? 0) > 0);
    return withRpm ?? fans[0] ?? null;
  }, [fans]);

  const entry = fanControl.entries.find((e) => e.fanId === cpuFan?.id);
  const currentMode: FanMode = entry?.mode ?? "bios";
  // Optimistic local mode for instant UI feedback
  const [optimisticMode, setOptimisticMode] = useState<FanMode | null>(null);
  const displayMode: FanMode = optimisticMode ?? currentMode;

  useEffect(() => {
    if (entry?.mode === "manual" && Number.isFinite(entry.manualPwm)) {
      setManualPwm(Math.round(entry.manualPwm));
    }
  }, [entry?.manualPwm, entry?.mode]);

  if (!cpuFan) return null;

  const blocked = disabled || !isAdmin;
  const pwmBusy = blocked || saving;
  const pwmPct = cpuFan.pwmPercent ?? 0;

  async function switchMode(mode: FanMode) {
    if (!cpuFan) return;
    // Instant UI update
    setOptimisticMode(mode);
    setSaving(true);
    try {
      if (mode === "bios") {
        const next = await resetFanControl(cpuFan.id);
        setFanControl(next);
        void message.success("已交给 BIOS 控制，风扇将在数秒内恢复固件曲线");
      } else if (mode === "curve") {
        const next = await setFanCurve(cpuFan.id, OPTIMIZED_CURVE);
        setFanControl(next);
        void message.success("已启用温控曲线（≤40°C 静音，≥85°C 全速）");
      } else if (mode === "manual") {
        const next = await setFanManual(cpuFan.id, manualPwm);
        setFanControl(next);
        void message.success(`已设置手动 PWM ${manualPwm}%`);
      }
    } catch (e) {
      void message.error(e instanceof Error ? e.message : String(e));
      // Rollback on failure
      setOptimisticMode(null);
    } finally {
      setSaving(false);
      setOptimisticMode(null);
    }
  }

  async function applyManualPwm(pwm: number) {
    if (!cpuFan) return;
    setManualPwm(pwm);
    if (currentMode === "manual") {
      setSaving(true);
      try {
        const next = await setFanManual(cpuFan.id, pwm);
        setFanControl(next);
      } catch (e) {
        void message.error(e instanceof Error ? e.message : String(e));
      } finally {
        setSaving(false);
      }
    }
  }

  return (
    <div
      style={{
        background: "#f9fafb",
        borderRadius: 8,
        padding: "14px 16px",
        border: "1px solid #f0f0f0",
      }}
    >
      {/* 顶部：风扇名 + 状态 + 模式切换 */}
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12 }}>
        <DashboardOutlined style={{ fontSize: 18, color: "#3388cc" }} />
        <div style={{ flex: 1 }}>
          <Text strong style={{ fontSize: 14 }}>
            {cpuFan.name.split(" / ").pop()}
          </Text>
          <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 4 }}>
            <Text style={{ fontSize: 20, fontWeight: 700, color: "#3388cc", fontVariantNumeric: "tabular-nums" }}>
              {cpuFan.rpm != null ? cpuFan.rpm : "--"}
              <span style={{ fontSize: 12, fontWeight: 400, color: "#6b7280", marginLeft: 3 }}>RPM</span>
            </Text>
            <span
              style={{
                fontSize: 11,
                fontWeight: 600,
                color: modeColor(displayMode),
                background: `${modeColor(displayMode)}18`,
                padding: "2px 8px",
                borderRadius: 10,
              }}
            >
              {modeLabel(displayMode)}
            </span>
            <Radio.Group
              value={displayMode}
              onChange={(e) => void switchMode(e.target.value)}
              disabled={blocked}
              size="small"
              optionType="button"
              buttonStyle="solid"
            >
              <Radio.Button value="bios">BIOS 自动</Radio.Button>
              <Radio.Button value="curve">温控曲线</Radio.Button>
              <Radio.Button value="manual">手动</Radio.Button>
            </Radio.Group>
          </div>
        </div>
      </div>

      {/* PWM 进度条 */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          marginBottom: 14,
          background: "#fff",
          borderRadius: 6,
          padding: "8px 12px",
          border: "1px solid #f0f0f0",
        }}
      >
        <Text type="secondary" style={{ fontSize: 11, flexShrink: 0, fontWeight: 500 }}>PWM</Text>
        <div style={{ flex: 1, position: "relative", height: 10, borderRadius: 5, background: "#f3f4f6" }}>
          <div
            style={{
              position: "absolute",
              left: 0,
              top: 0,
              height: "100%",
              width: `${Math.round(pwmPct)}%`,
              borderRadius: 5,
              background: pwmPct >= 80
                ? "linear-gradient(90deg, #f97316, #ef4444)"
                : pwmPct >= 50
                  ? "linear-gradient(90deg, #3388cc, #f97316)"
                  : "linear-gradient(90deg, #22c55e, #3388cc)",
              transition: "width 0.3s ease",
            }}
          />
          {/* Right-pointing arrow on the bar */}
          <div
            style={{
              position: "absolute",
              top: "50%",
              left: `${Math.round(pwmPct)}%`,
              transform: "translate(-50%, -50%)",
              transition: "left 0.3s ease",
            }}
          >
            <svg width="12" height="12" viewBox="0 0 12 12" style={{ display: "block" }}>
              <path d="M2 1L10 6L2 11z" fill={pwmPct >= 80 ? "#ef4444" : pwmPct >= 50 ? "#f97316" : "#3388cc"} />
            </svg>
          </div>
        </div>
        <Text
          style={{
            fontSize: 13,
            fontWeight: 700,
            fontVariantNumeric: "tabular-nums",
            width: 40,
            textAlign: "right",
            color: pwmPct >= 80 ? "#ef4444" : pwmPct >= 50 ? "#f97316" : "#3388cc",
          }}
        >
          {pwmPct.toFixed(0)}%
        </Text>
      </div>

      {/* 手动模式 */}
      {displayMode === "manual" && (
        <ManualControl
          manualPwm={manualPwm}
          blocked={pwmBusy}
          onPwmChange={setManualPwm}
          onPwmApply={applyManualPwm}
        />
      )}

      {/* 曲线模式 */}
      {displayMode === "curve" && (
        <CurveDisplay cpuTemp={cpuTemp} />
      )}
    </div>
  );
}

/* ─── Manual Control ─────────────────────────────────────────────────── */

function ManualControl({
  manualPwm,
  blocked,
  onPwmChange,
  onPwmApply,
}: {
  manualPwm: number;
  blocked: boolean;
  onPwmChange: (v: number) => void;
  onPwmApply: (v: number) => void;
}) {
  const presets = [20, 40, 60, 80];

  return (
    <div
      style={{
        background: "#ffffff",
        borderRadius: 6,
        padding: "10px 14px",
        border: "1px solid #e5e7eb",
      }}
    >
      {/* Slider with preset dots */}
      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <Text type="secondary" style={{ fontSize: 12, flexShrink: 0 }}>PWM</Text>
        <Slider
          min={0}
          max={100}
          value={manualPwm}
          onChange={(v) => onPwmChange(v)}
          onChangeComplete={(v) => void onPwmApply(v)}
          disabled={blocked}
          style={{ flex: 1 }}
          marks={Object.fromEntries(presets.map((p) => [p, ""]))}
          tooltip={{ formatter: (v) => `${v}%` }}
        />
        <Text style={{ fontSize: 13, fontWeight: 600, fontVariantNumeric: "tabular-nums", width: 40, textAlign: "right" }}>
          {manualPwm}%
        </Text>
      </div>

      {/* Preset buttons as compact chips */}
      <div style={{ display: "flex", gap: 6, marginTop: 6, paddingLeft: 32 }}>
        {presets.map((preset) => {
          const active = manualPwm === preset;
          return (
            <button
              key={preset}
              onClick={() => void onPwmApply(preset)}
              disabled={blocked}
              style={{
                padding: "3px 12px",
                fontSize: 11,
                fontWeight: 600,
                color: active ? "#fff" : "#6b7280",
                background: active ? "#3388cc" : "transparent",
                border: active ? "1px solid #3388cc" : "1px solid #e5e7eb",
                borderRadius: 12,
                cursor: blocked ? "not-allowed" : "pointer",
                transition: "all 0.12s ease",
              }}
            >
              {preset}%
            </button>
          );
        })}
      </div>
    </div>
  );
}

/* ─── Curve Display with live temperature indicator ───────────────────── */

function CurveDisplay({ cpuTemp }: { cpuTemp: number | null }) {
  const curveMin = OPTIMIZED_CURVE[0].tempC;
  const curveMax = OPTIMIZED_CURVE[OPTIMIZED_CURVE.length - 1].tempC;
  const currentPwm = cpuTemp != null ? interpolateCurve(OPTIMIZED_CURVE, cpuTemp) : null;

  // SVG curve visualization
  const svgW = 360;
  const svgH = 120;
  const padL = 36;
  const padR = 14;
  const padT = 14;
  const padB = 24;
  const plotW = svgW - padL - padR;
  const plotH = svgH - padT - padB;

  const toX = (t: number) => padL + ((t - curveMin) / (curveMax - curveMin)) * plotW;
  const toY = (p: number) => padT + plotH - (p / 100) * plotH;

  const pathD = OPTIMIZED_CURVE.map((pt, i) =>
    `${i === 0 ? "M" : "L"} ${toX(pt.tempC).toFixed(1)} ${toY(pt.pwm).toFixed(1)}`
  ).join(" ");

  // Gradient fill area under curve
  const areaD = `${pathD} L ${toX(curveMax).toFixed(1)} ${toY(0).toFixed(1)} L ${toX(curveMin).toFixed(1)} ${toY(0).toFixed(1)} Z`;

  // Temperature zones for background coloring
  const zoneColors = [
    { from: 0, to: 40, color: "#22c55e", label: "静音" },
    { from: 40, to: 70, color: "#eab308", label: "均衡" },
    { from: 70, to: 85, color: "#ef4444", label: "高性能" },
  ];

  return (
    <div
      style={{
        background: "#ffffff",
        borderRadius: 8,
        padding: "14px 16px",
        border: "1px solid #e5e7eb",
      }}
    >
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 10 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Text style={{ fontSize: 12, fontWeight: 600, color: "#374151" }}>温控曲线</Text>
          {/* Zone legend */}
          <div style={{ display: "flex", gap: 6 }}>
            {zoneColors.map((z) => (
              <span
                key={z.label}
                style={{
                  fontSize: 9,
                  color: z.color,
                  background: `${z.color}14`,
                  padding: "1px 6px",
                  borderRadius: 8,
                  fontWeight: 500,
                }}
              >
                {z.label}
              </span>
            ))}
          </div>
        </div>
        {cpuTemp != null && currentPwm != null && (
          <div
            style={{
              fontSize: 11,
              fontWeight: 600,
              fontVariantNumeric: "tabular-nums",
              color: "#fff",
              background: cpuTemp >= 70 ? "#ef4444" : cpuTemp >= 40 ? "#f97316" : "#22c55e",
              padding: "2px 10px",
              borderRadius: 10,
            }}
          >
            {cpuTemp.toFixed(0)}°C → {currentPwm.toFixed(0)}%
          </div>
        )}
      </div>

      {/* SVG Curve */}
      <svg
        viewBox={`0 0 ${svgW} ${svgH}`}
        style={{ width: "100%", height: 120, display: "block" }}
      >
        <defs>
          <linearGradient id="curveGradient" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor="#22c55e" stopOpacity={0.12} />
            <stop offset="47%" stopColor="#eab308" stopOpacity={0.10} />
            <stop offset="100%" stopColor="#ef4444" stopOpacity={0.12} />
          </linearGradient>
          <linearGradient id="strokeGradient" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stopColor="#22c55e" />
            <stop offset="47%" stopColor="#3388cc" />
            <stop offset="100%" stopColor="#ef4444" />
          </linearGradient>
        </defs>

        {/* Background zone bands */}
        {zoneColors.map((z) => (
          <rect
            key={z.from}
            x={toX(z.from)}
            y={padT}
            width={toX(z.to) - toX(z.from)}
            height={plotH}
            fill={z.color}
            opacity={0.03}
          />
        ))}

        {/* Horizontal grid lines */}
        {[0, 25, 50, 75, 100].map((p) => (
          <line
            key={p}
            x1={padL}
            y1={toY(p)}
            x2={svgW - padR}
            y2={toY(p)}
            stroke="#e5e7eb"
            strokeWidth={0.5}
            strokeDasharray={p === 0 ? undefined : "2 3"}
          />
        ))}

        {/* Area fill with gradient */}
        <path d={areaD} fill="url(#curveGradient)" />

        {/* Curve line with gradient */}
        <path
          d={pathD}
          fill="none"
          stroke="url(#strokeGradient)"
          strokeWidth={2.5}
          strokeLinejoin="round"
          strokeLinecap="round"
        />

        {/* Curve points */}
        {OPTIMIZED_CURVE.map((pt) => {
          const isActive = cpuTemp != null && Math.abs(pt.tempC - cpuTemp) < 3;
          return (
            <circle
              key={pt.tempC}
              cx={toX(pt.tempC)}
              cy={toY(pt.pwm)}
              r={isActive ? 4 : 3}
              fill="#fff"
              stroke={pt.tempC >= 70 ? "#ef4444" : pt.tempC >= 40 ? "#eab308" : "#22c55e"}
              strokeWidth={isActive ? 2 : 1.5}
            />
          );
        })}

        {/* Current temperature indicator */}
        {cpuTemp != null && currentPwm != null && cpuTemp >= curveMin && cpuTemp <= curveMax && (
          <>
            {/* Vertical line */}
            <line
              x1={toX(cpuTemp)}
              y1={toY(currentPwm) + 6}
              x2={toX(cpuTemp)}
              y2={toY(0)}
              stroke="#f97316"
              strokeWidth={1}
              strokeDasharray="3 2"
              opacity={0.5}
            />
            {/* Horizontal line to Y axis */}
            <line
              x1={padL}
              y1={toY(currentPwm)}
              x2={toX(cpuTemp) - 6}
              y2={toY(currentPwm)}
              stroke="#f97316"
              strokeWidth={1}
              strokeDasharray="3 2"
              opacity={0.4}
            />
            {/* Glow effect */}
            <circle
              cx={toX(cpuTemp)}
              cy={toY(currentPwm)}
              r={8}
              fill="#f97316"
              opacity={0.15}
            />
            {/* Main dot */}
            <circle
              cx={toX(cpuTemp)}
              cy={toY(currentPwm)}
              r={5}
              fill="#f97316"
              stroke="#fff"
              strokeWidth={2}
            />
          </>
        )}

        {/* X axis labels */}
        {OPTIMIZED_CURVE.filter((_, i) => i % 2 === 0 || i === OPTIMIZED_CURVE.length - 1).map((pt) => (
          <text
            key={pt.tempC}
            x={toX(pt.tempC)}
            y={svgH - 4}
            textAnchor="middle"
            fontSize={9}
            fill="#9ca3af"
            fontFamily="system-ui"
          >
            {pt.tempC}°
          </text>
        ))}

        {/* Y axis labels */}
        {[0, 50, 100].map((p) => (
          <text
            key={p}
            x={padL - 6}
            y={toY(p) + 3}
            textAnchor="end"
            fontSize={9}
            fill="#9ca3af"
            fontFamily="system-ui"
          >
            {p}%
          </text>
        ))}
      </svg>

      {/* Footer */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: 6 }}>
        <Text type="secondary" style={{ fontSize: 10 }}>
          ≤40°C 静音保护 · ≥85°C 全速
        </Text>
        <Text type="secondary" style={{ fontSize: 10 }}>
          实时插值
        </Text>
      </div>
    </div>
  );
}
