# 扫描清理与稳定性重构任务

- [x] 重构扫描计划、逐文件清理、取消与结构化结果；更新清理前端。
- [x] 删除 Codex 会话扫描、解析、索引、删除和对应 UI（保留独立 MCP 配置功能）。
- [x] 拆分高负载采集链路，区分无数据、延迟与真实零值；完成合成回归及真实 helper 压力测试。
- [x] 排查偶发退出，修复已确认的风险路径并增加诊断。
- [x] 完成 Rust、前端、硬件合成测试、构建和隔离浏览器交互验证。
- [x] 启动原生开发构建，确认 helper ready、悬浮窗及真实风扇读数。
- [x] 验证原生主窗口关闭后进程保留、重复启动恢复原窗口和页面状态。
- [x] 复验退出残留进程修正并更新最终 Release 构建。
- [x] 完成管理员权限下 CPU 温度、风扇及磁盘的高负载采集验证。
- [x] 验证最终 Release 构建的最小化恢复。

约束：不运行真实垃圾删除；不删除用户 AI 会话；不将未经复现的闪退归因为已确认根因。

## 实现记录

### 扫描和清理

- 扫描按文件形成仅由后端保存的计划。清理消费扫描 ID，仅处理审核时已记录的文件，不再递归删除目录。
- 删除前复核句柄身份、大小、时间、父链及重解析点；变化、缺失、占用和取消分别统计。保留期按文件判断，无法取得必要时间时跳过。
- 排除目录作用于整个子树；硬链接按真实释放情况统计，避免重复计算释放空间。
- 浏览器占用检测覆盖默认配置和自定义 user-data-dir；进程采样只读取所需字段。
- 扫描和清理状态由应用级 store 保存，切换页面不会重启任务；支持停止清理、保留部分结果，完成后由用户重新扫描。
- 增加嵌套排除、硬链接、父目录消失、junction 祖先替换、浏览器配置路径的回归覆盖。

### 会话管理

- 移除 Codex 会话扫描、解析、分页、索引及删除代码和对应界面能力；更新类型、测试与说明。
- Claude/Gemini 会话资料库保留；Codex MCP 配置入口保留。没有删除用户会话数据。

### 硬件与退出

- helper 将磁盘与其他传感器分别采集，通过不可变快照响应请求，慢磁盘不再阻塞快照协议。
- 快照分别记录传感器和磁盘年龄；后端缓存驻留时间计入年龄。前端超过 5 秒提示延迟、超过 30 秒显示未知，恢复采样后更新。真实 0 RPM 不当成缺失。
- 自动风扇控制仍按 5 秒新鲜度判定；30 秒显示窗口不用于控制。
- helper 退出先停止采集再关闭设备；合成测试验证磁盘阻塞期间其他采集继续、关闭等待设备线程安全结束。
- 移除 overlay 对 Explorer 的 SetParent 依赖，保留顶层悬浮窗并维护位置；release 改用 unwind，广播溢出后继续订阅。
- 主动退出幂等化；记录关闭超时、窗口销毁与运行时退出请求；防止无主动退出码的窗口丢失直接结束托盘程序。
- 重复启动入口补充 `unminimize()`，窗口操作失败进入已有诊断日志。
- 历史日志中 Explorer/WebView2 异常先后出现，只作为相关线索，未确认所有历史闪退的唯一根因。

## 验收结果（2026-11-04）

| 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked --quiet` | 147 passed，1 ignored |
| `npm test` | 14 文件、73 passed；最终复验通过 |
| `dotnet run --project tests/hw-helper/HwHelper.Tests.csproj -c Release` | 22 hardware regressions passed |
| `npm run build` | 通过；现有主 bundle 大于 500 kB 的提示仍存在 |
| `npx --no-install tsc -p tests/ui/tsconfig.json` | 通过 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 通过 |
| `git diff --check` | 通过；Git 提示部分文件将转换 CRLF |
| `cargo run --manifest-path src-tauri/Cargo.toml --bin export_bindings --locked` | 类型绑定已同步 |
| `cargo build --manifest-path src-tauri/Cargo.toml --bin syspulse --features custom-protocol --locked` | 原生开发构建通过；不是 release 安装包验收 |
| `cargo build --release --manifest-path src-tauri/Cargo.toml --bin syspulse --features custom-protocol --locked` | 23:54 最终构建通过，包含最小化恢复和退出残留修正 |
| helper Release、自包含单文件 publish | 通过，产物位于 `target/hw-validation/helper/` |

隔离浏览器验收：扫描和清理期间切页后返回，调用次数仍各一次；清理结束无自动重扫；停止清理保留部分报告；会话资料库没有 Codex，MCP 仍有 Codex；约 800×560 的最小窗口无横向溢出；检查时控制台无 error/warn。

硬件页面冷启动自动绑定正常，模拟 62°C 和 0 RPM 正常显示；等待超过 5 秒出现延迟提示，超过 30 秒变为未知；再次发布事件后恢复 62°C/900 RPM。验收中重复 import 热更新前后的 fixture 曾重置模拟监听，重启 Vite 并加载页面实际模块后排除该测试环境问题。

真实压力测试命令：`python tests/hw-helper/probe.py target/hw-validation/helper/hw-helper.exe --stress`。5 秒基线、12 秒负载、5 秒恢复；只请求 snapshot/shutdown，磁盘写入限定于测试自建文件。

| 阶段 | 请求数 | 最大响应耗时 |
| --- | ---: | ---: |
| baseline | 20 | 50.73 ms |
| load | 43 | 2.04 ms |
| recovery | 20 | 4.57 ms |

负载阶段 CPU 最高 100%，最大传感器年龄 3464 ms，全部请求成功且无超时，helper 正常退出。原始记录在 `target/hw-validation/helper-probe.json`。本次非管理员测试 CPU 温度一直为空，磁盘传感器列表为空；不能据此声称真实 CPU 温度或磁盘传感器在满载下已经验证。风扇转速及 GPU 项目在实际快照中存在，阻塞路径另由合成测试覆盖。

23:27 直接启动 `target/debug/syspulse.exe`，核实进程路径、版本 0.1.30 和 helper ready；悬浮窗显示 CPU、内存、网络和约 963–964 RPM 的真实风扇转速，CPU 温度为未知。该桌面构建早于最后一处取消失败文案修正，最终前端源码已重新构建并通过测试。日志出现一次 GitHub 更新检查网络失败，不代表程序退出。

## 最终复验与现场状态

- 用户重新授权桌面验收后，已从可见主窗口验证关闭隐藏、同一进程存活、重复启动恢复同一窗口，清理页面的扫描结果保留。
- 23:42 在悬浮窗右键菜单选择退出：helper 已结束，但主进程仍残留。日志的 `runtime exit requested code=Some(0)` 只证明退出请求，不能当成进程终止证据。已明确更正先前的过早判断。
- 对残留开发进程进行了非侵入线程栈检查：主线程停在 `GetMessageW -> tao::EventLoop::run_return`；证据在 `target/hw-validation/exit-stack-local.txt`。收集完成后仅结束这一个已请求退出、helper 已结束的测试进程。
- 修正：主动清理结束后的 Windows `ExitRequested` 回调投递 `WM_QUIT`，唤醒消息循环并继续正常的 `LoopDestroyed` / Tauri 清理；主动退出期间不再把窗口关闭改为隐藏。23:48 开发构建通过并启动新进程；用户再次按 Esc 后停止自动桌面操作，随后由用户手动选择退出。
- 23:53 复验通过：日志依次记录 `explicit shutdown requested`、`sampler shutdown`、`runtime exit requested code=Some(0)` 和最终 `SysPulse runtime exited`，耗时约 0.52 秒；后台确认主进程 PID 41320 和 helper PID 32608 均已结束。证据在 `target/hw-validation/desktop-exit-fixed.log`。最后一行 WebView2 注销窗口类警告不阻止进程正常结束。
- 悬浮窗右键菜单已实际打开；此前自动化未立即观察到菜单不构成点击穿透的确认问题。未改透明背景，也未重启 Explorer。
- 用户批准并手动确认 UAC 后，执行管理员压力验收；脚本只请求 snapshot/shutdown，不调整风扇。原始记录在 `target/hw-validation/admin-probe/helper-probe.json`，通过 `--output-dir` 与此前非管理员数据分开保存。测试退出码 0，无残留 helper 和测试负载文件。
- 管理员基线 20 次（首帧尚无采样，后续 19 次均有温度/风扇/磁盘），负载 45 次、恢复 20 次。负载阶段 CPU 最高 100%，45 次均有 CPU 温度（54–70°C）、风扇及磁盘数据；最大响应 2.56 ms，传感器年龄最大 3541 ms、磁盘年龄最大 2974 ms。恢复阶段最大响应 1.29 ms，20 次数据完整。磁盘负载受脚本独立文件和总写入量限制，未声称整盘达到 100% 饱和；不可返回的磁盘调用由合成测试覆盖。
- 最终 Release 可执行文件位于 `target/release/syspulse.exe`，helper 已配齐。SHA-256：`0EE3CC9C162FFA9F96ED24846B0A9750750E4A242BA39640EFA917F3B9983F8A`。未替换安装版、未提交或发布；Vite 已停止。
- 23:58 启动最终 Release（PID 22712），用户从托盘打开并最小化主窗口；23:59 再次启动同一路径，第二实例 PID 23856 以 0 退出，后台确认原进程继续运行。用户明确确认主窗口自动恢复，最小化恢复验收通过。该 Release 目前保留运行。
- 短时压力和可复现退出路径已验证；历史偶发闪退未做长期复现，后续以新增诊断日志继续观察，不宣称所有历史闪退原因都已穷尽。

任务于 2026-11-05 完成：约定代码改动、构建、回归及本机可行运行验证均已完成。长期运行观察属于交付后的验证边界，不作为已经完成的测试结果。
