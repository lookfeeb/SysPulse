import { useMemo, useState } from "react";
import { Button, Empty, Pagination, Popover } from "antd";
import { ApiOutlined, CheckOutlined, FilterOutlined, InfoCircleOutlined } from "@ant-design/icons";
import type { BoardInterface } from "@/bindings";
import { capabilityLabel, connectionLabel, filterPorts, isUsbPort, portShapeLabel, supportedRate, usbShape, usbVersion } from "@/utils/boardInterfaces";
import "./BoardInterfaces.css";

const categories = [
  ["all", "全部"], ["usb", "USB"], ["pcie", "PCIe 链路"],
  ["firmware-slot", "扩展插槽"], ["network", "网卡"], ["firmware-connector", "固件接口"],
] as const;
const sources: Record<string, string> = {
  usb: "USB 实时采集", pcie: "PCIe 设备", network: "系统网卡",
  "firmware-slot": "固件 · 开机状态", "firmware-connector": "固件清单",
};
const PAGE_SIZE = 10;

function InterfaceRow({ port }: { port: BoardInterface }) {
  const usb = isUsbPort(port);
  const protocol = usb ? usbVersion(port) : port.kind;
  const maximum = supportedRate(port);
  return <tr data-port-id={port.id}>
    <th scope="row">
      <span className="board-ports__name">{port.name || "未命名接口"}</span>
      <span className={`board-ports__detail ${usb && usbShape(port) === "c" ? "board-ports__type-c" : ""}`}>
        {usb ? portShapeLabel(port) : port.connector || "形态未报告"}
      </span>
      {port.usb && <span className="board-ports__detail">
        {port.usb.topologyNote}{port.usb.userConnectable === false ? " · 内部连接" : ""}
      </span>}
    </th>
    <td>
      <span>{protocol || "未报告"}</span>
      {port.source === "usb" && <span className="board-ports__detail">
        {protocol === "USB 3.x" ? "系统协议 · 细分版本未知" : "系统协议"}
      </span>}
    </td>
    <td className="board-ports__speed">
      <div><span className="board-ports__speed-label">当前协商</span><span>{port.currentSpeed || (port.status === "disconnected" ? "未连接" : "未报告")}</span></div>
      {maximum && <div className="board-ports__detail" title={port.usb?.maximumSpeedSource ?? undefined}>
        <span>{capabilityLabel(port)}</span><span>{maximum}</span>
      </div>}
      {port.usb?.deviceCapability && <div className="board-ports__detail" title={port.usb.deviceCapabilitySource ?? undefined}>
        <span>设备 USB 能力</span><span>{port.usb.deviceCapability}</span>
      </div>}
    </td>
    <td><span className={`board-ports__status board-ports__status--${port.status}`}>
      <i aria-hidden="true" />{connectionLabel(port.status)}
    </span></td>
    <td>
      <span className="board-ports__device">{port.device || (port.status === "disconnected" ? "暂无设备" : "未报告")}</span>
      <span className="board-ports__detail">{sources[port.source] || "来源未报告"}</span>
    </td>
  </tr>;
}

export default function BoardInterfaces({ ports }: { ports: BoardInterface[] }) {
  const [source, setSource] = useState("all");
  const [connectedOnly, setConnectedOnly] = useState(false);
  const [page, setPage] = useState(1);
  const counts = useMemo(() => {
    const values: Record<string, number> = { all: ports.length };
    for (const p of ports) values[p.source] = (values[p.source] ?? 0) + 1;
    return values;
  }, [ports]);
  const typeCCount = ports.filter(p => p.source === "usb" && usbShape(p) === "c").length;
  const connectedCount = ports.filter(p => p.status === "connected").length;
  const filtered = useMemo(() => filterPorts(ports, source, connectedOnly), [ports, source, connectedOnly]);
  const currentPage = Math.min(page, Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  const shown = filtered.slice((currentPage - 1) * PAGE_SIZE, currentPage * PAGE_SIZE);
  function changeCategory(next: string) {
    setSource(next); setPage(1);
  }
  const help = <div className="board-ports__help">
    <p>从当前电脑的固件、USB 控制器、PCIe 设备和网卡自动读取，不使用特定主板的预设规格。</p>
    <p>系统明确报告且双向对应的 USB 伴随端口会合并展示；端口组不一定是机箱外部插孔。USB-C 是形态，USB 3.x 不能直接区分最高 5、10 或 20 Gb/s。</p>
    <p>系统 USB 通道上限只在协议和伴随映射完整时显示，不代表主板插孔的设计上限。设备 USB 能力来自设备描述符，也不代表主板能力。未可靠获取的上限不显示。</p>
    <p>PCIe 的设备链路上限不等于主板插槽上限；GT/s 为每通道传输率，×N 为通道数。固件占用是开机状态。系统未提供的信息显示“未报告”。</p>
    <p>USB、PCIe 和网卡约每 5 秒重新采集，热插拔状态随下一次采集自动更新，并非插拔事件即时推送。</p>
  </div>;
  return <section className="board-ports" data-testid="board-interfaces" aria-label="主板与扩展接口">
    <div className="board-ports__header">
      <div><h3><ApiOutlined />主板与扩展接口</h3><p>自动采集当前电脑 · 实时数据约每 5 秒更新</p></div>
      <Popover content={help} title="接口数据说明" trigger="click">
        <Button type="text" icon={<InfoCircleOutlined />} aria-label="查看接口说明" />
      </Popover>
    </div>
    <div className="board-ports__summary">
      <span>已采集 <b>{ports.length}</b> 条</span>
      <span>USB-C 记录 <b>{typeCCount}</b></span>
      <button type="button" className="board-ports__connected-filter" aria-pressed={connectedOnly}
        title={connectedOnly ? "点击取消已连接筛选" : "点击筛选当前分类中已连接的接口"}
        onClick={() => { setConnectedOnly(value => !value); setPage(1); }}>
        {connectedOnly ? <CheckOutlined aria-hidden /> : <FilterOutlined aria-hidden />}
        <span>当前已连接 <b>{connectedCount}</b></span>
      </button>
    </div>
    <nav className="board-ports__categories" aria-label="接口分类">
      {categories.map(([key, label]) => <button key={key} type="button" aria-pressed={source === key}
        onClick={() => changeCategory(key)}>{label}<span>{counts[key] ?? 0}</span></button>)}
    </nav>
    {shown.length ? <div className="board-ports__table-scroll" role="region" aria-label="接口数据列表" tabIndex={0}>
      <table className="board-ports__table">
        <caption className="board-ports__sr-only">当前电脑自动采集的接口数据</caption>
        <colgroup><col style={{ width: "25%" }} /><col style={{ width: "17%" }} /><col style={{ width: "24%" }} /><col style={{ width: "12%" }} /><col style={{ width: "22%" }} /></colgroup>
        <thead><tr><th scope="col">接口 / 形态</th><th scope="col">协议 / 类型</th><th scope="col">速率</th><th scope="col">连接状态</th><th scope="col">设备 / 来源</th></tr></thead>
        <tbody>{shown.map(p => <InterfaceRow key={p.id} port={p} />)}</tbody>
      </table>
    </div> : <div className="board-ports__empty"><Empty image={Empty.PRESENTED_IMAGE_SIMPLE}
      description={ports.length ? "没有匹配的接口" : "暂未收到接口数据"} />
      {ports.length > 0 && <Button size="small" onClick={() => { changeCategory("all"); setConnectedOnly(false); }}>显示全部接口</Button>}
    </div>}
    <div className="board-ports__footer">
      <span>共 {filtered.length} 条记录 · {ports.some(p => (p.usb?.logicalPortCount ?? 1) > 1) ? "已合并 USB 伴随记录" : "包含集线器逻辑端口"}</span>
      <Pagination size="small" current={currentPage} pageSize={PAGE_SIZE} total={filtered.length}
        onChange={setPage} showSizeChanger={false} hideOnSinglePage showLessItems />
    </div>
  </section>;
}
