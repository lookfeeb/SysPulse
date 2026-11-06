# 隔离界面验收

本入口使用 Tauri 的 `mockIPC` / `mockWindows` 加载真实组件；未知命令报错，模拟扫描、删除和 OAuth 只改变内存数据，刷新后恢复。它不进入生产构建，也不代替真实桌面、文件系统、硬件或 OAuth 服务验收。

在仓库根运行：

```powershell
node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 5175
```

打开 `http://127.0.0.1:5175/tests/ui/index.html`。类型检查命令为 `npx --no-install tsc -p tests/ui/tsconfig.json`；其他检查见[项目 README](../../README.md)。

执行以下控制台脚本前，若刚修改源码触发了 HMR，请重启 Vite 并刷新页面。热更新入口可能带有 `?t=...`；再导入不带时间戳的 fixture 会重新执行 mockIPC、重置监听，导致验收状态失真。

## 场景

显示顺序：打开 `?overlay-order` 并进入“通用”，提供 6 个模拟显示项。控制台执行 `await (await import('/tests/ui/overlay-order-check.ts')).checkOverlayOrder()`，检查点击阈值、快速跨项拖动、稳定落点、高亮、单次保存、多指隔离、取消、延迟/失败回退和外部配置变更。该脚本使用浏览器 PointerEvent 与模拟 IPC；还需用真实鼠标/键盘检查捕获、跟手效果和焦点。分别用宽窗口单行、窄窗口换行验证；空格选中，左右移动，上下跨行，Home/End 到首尾，空格/Enter 保存，Esc/Tab 取消。保存期间禁用重复编辑，键盘焦点保留。

流量与自启：导入 `trafficFixture`、`autostartFixture`（`/tests/ui/fixtures.ts`）。默认提供最近 31 天的合成流量；修改 `trafficFixture.rows` 可验证每秒刷新，`fail` / `delayMs` 可验证失败保留与慢请求。`calls` / `exports` 记录查询和导出参数，导出只记录内存调用。`?autostart-enabled` 从开启状态启动；`autostartFixture.delayMs` 默认 600 ms，反复切入通用页时已确认开关不应加载闪动，真实启停操作仅修改模拟状态。

`?cleanup-paths` 使用带扩展前缀的盘符与 UNC 路径；路径显示应去掉前缀，点击后 `cleanupFixture.revealCalls` 保存原始路径，勾选不变。设置 `cleanupFixture.failNextReveal = true` 可检查定位失败提示。

`/tests/ui/overlay.html` 加载真实悬浮窗，暂挂菜单初始化。导入 `overlayFixture`（`/tests/ui/overlay-fixture.ts`）检查 `showCalls`：单击不增加、双击增加一次；调用 `finishMenu()` 后仍然有效，右键增加 `popupCalls`。

菜单初始化完成后，调用 `await overlayFixture.publish(38.6 * 1024 ** 2, 7.22 * 1024 ** 2)` 检查完整网速单位与两行布局；`sizes` 保存窗口尺寸请求，`configure(items)` 切换模拟显示项。可用 `publish(0, 128, 73, 0)` 验证数值变短后窗口收缩，所有调用仅影响隔离页面。

加入 `disk-read` 显示项后检查“硬”“盘”竖排、读取/写入 SVG 图标和完整单位。`regionCalls` 记录悬停区域：按下鼠标后最后一次 `regions` 为空，后续刷新仍为空，离开后恢复。自绘提示面板在 Rust 原生测试中检查；设置 `SYSPULSE_TOOLTIP_PREVIEW` 为 BMP 路径可导出同一绘制函数的预览。

| 地址参数 | 数据与检查重点 |
| --- | --- |
| 无参数 | 一万条会话、80 条消息分页、12 个概览采样点、模拟清理和 OAuth；检查虚拟列表、键盘导航、详情分页、删除确认及授权状态 |
| `?session-sidebar` | 同名不同路径项目、空项目、只读备份、链接前缀与长标题；检查项目下拉选择、品牌图标、全部来源会话、页码恢复和慢请求切换 |
| `?config-failure` | 配置请求先失败后恢复；检查错误提示、首次失败诊断和“重新读取” |
| `?cleanup-background` | 扫描和清理等待显式完成；检查切页后进度延续、报告保留和取消 |

在 1000×680 和 800×560 视口检查页面与弹窗无横向溢出，查看控制台错误。通用页现有 `InputNumber.addonAfter` 弃用提示见项目 README。

## 会话控制

浏览器控制台加载：

```javascript
const { sessionFixture } = await import('/tests/ui/fixtures.ts');
```

`listDelayMs`、`loadDelayMs` 控制延迟，`failNextList`、`failNextLoad` 注入一次失败；`listCalls`、`loadCalls`、`deleteCalls`、`remaining` 用于核对请求和数据。

检查慢加载期间切换项目不会被旧响应覆盖；删除取消不改数据，重复确认只提交一次；删除成功但刷新失败时，已删除行仍应消失。只读记录保持只读。项目下拉框只允许选择；侧栏不显示搜索框、项目路径、本机会话数、来源筛选或结果栏。列表快捷键包括 ↓、End 和 Enter。

## 历史日期查询

进入“流量历史”，日期面板和月份选择面板应显示 `1月` 至 `12月`，星期使用中文。
加载 `const { trafficFixture } = await import('/tests/ui/fixtures.ts')`，向 `rows` 加入一条 `2001-01-01` 的记录；清空开始、结束日期后点击“查询”，应显示“全部历史记录”，统计包含这条旧记录，最后一次 `calls` 的 `from`、`to` 均为 `null`。
切换按月后查询、等待实时刷新、切页返回及导出 CSV，仍应保留空日期范围；`exports` 中的查询参数同样为 `null`。仅指定一端日期时，只应用该端限制。

## 后台清理控制

在 `?cleanup-background` 场景加载：

```javascript
const { cleanupFixture } = await import('/tests/ui/fixtures.ts');
```

1. 点击开始扫描，调用 `await cleanupFixture.progress(123)`；切到概览，再调用 `await cleanupFixture.progress(456)`。返回清理页仍为扫描中，`scanCalls` 为 1。
2. 调用 `cleanupFixture.finishScan()`；确认模拟清理后切页，`cleanCalls` 为 1。
3. 调用 `cleanupFixture.finishClean()`；返回清理页显示已清理 512 B、保留锁定文件的分目录报告。不会自动发起第二次扫描。
4. 再次主动扫描后报告仍保留；清理期间点击“停止清理”应保存部分结果。`finishScan(true)` 可模拟扫描取消结果。

## 硬件控制

```javascript
const { hardwareFixture } = await import('/tests/ui/fixtures.ts');
await hardwareFixture.publish();
```

CPU 温度应为 62°C，缺失占用显示未知；等待 6 秒后读数标记为延迟并停用风扇控制，超过 30 秒显示未知，再次发布后恢复。

`publish(fans)` 接受模拟 `FanHw[]`；`fanWrites` 记录请求，`fanWriteDelayMs` 控制延迟，`failNextFanWrite` 注入失败。检查只有唯一明确 CPU 风扇时才默认选择、0/未知/高速读数可区分、控制关联未知时只读、传感器消失不自动替换，以及重复操作去重和失败回退。所有写入仅修改模拟状态。

## MCP 缓存与目录弹窗

在控制台加载 `const { mcpFixture, cleanupFixture } = await import('/tests/ui/fixtures.ts')`。

- 启动后 `mcpFixture.listCalls` 应包含三个客户端各一次；反复进入 AI 管理、切换 MCP 客户端或会话页后计数不变；每两小时静默更新一次。点击“刷新状态”后增加三个读取请求。
- `mcpFixture.oauthCalls` 在切页或发布 `mcp-tokens-updated` 事件后不增加。启停服务器只增加 `toggleCalls`，界面与统计立即更新，切页后保留。
- 设 `mcpFixture.failNextRead = true` 后手动刷新，旧列表应保留；切页不重试，再手动刷新可恢复。
- 向 `cleanupFixture.directoryChoices` 放入合成路径后，“添加目录”和“替换目录”的选择框返回该路径。检查空态、长路径、16 项上限、取消移除及保存后重新打开；全部操作只改变模拟数据和此测试页面的本地存储。
