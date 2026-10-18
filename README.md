# SysPulse

Windows 桌面系统监控工具，使用 Tauri 2、React 19、TypeScript 和 .NET 8。提供任务栏悬浮窗、硬件监控、流量历史、磁盘清理，以及本地 AI 客户端管理。

## 功能

- **悬浮窗与概览**：显示网速、CPU、内存、温度、硬盘读写和风扇转速；支持显示项排序、透明度和任务栏贴合。图表按实际采样时间展示最近的数据。
- **硬件监控**：采集 CPU、GPU、内存、磁盘、主板和风扇数据。区分未知值与 `0 RPM`；过期快照停止显示。
- **风扇控制**：提供固件自动、手动 PWM 和 CPU 温控曲线。只有确认转速与控制通道关联后才允许写入；CPU 温度缺失、过期、达到 90°C 或连续写入失败时触发保护并尝试恢复固件控制。看门狗周期为 2 秒。
- **磁盘清理**：检查用户目录、固定磁盘和开发工具缓存，支持自定义项目根。扫描、清理和进度独立于页面，切换页面后继续运行；删除前复核文件身份、时间、保留期及重解析点，跳过占用或变化的文件。
- **AI 管理**：管理 Kiro、Codex、Claude CLI 的 MCP 配置与 OAuth；浏览、搜索、分页、导出及按来源能力删除 Kiro、Codex、Claude、Antigravity、Gemini 会话。包含本地 OAuth 代理、凭据加密和 Token 刷新。
- **流量与设置**：按日/月查询流量并导出 CSV；固定采样间隔为 500～5000 ms，自适应采样为 500～2000 ms。支持 Windows 计划任务自启、托盘操作和应用更新。

传感器可用性和控制能力取决于硬件、驱动及权限。无法确认的数据与控制关联保持未知或只读。

## 开发环境

| 依赖 | 要求 |
| --- | --- |
| 系统 | Windows 10 1809+ / Windows 11，WebView2 Runtime |
| Node.js | 推荐 22.12+；Vite 也支持 20.19+ |
| Rust | stable MSVC 工具链；依赖约束见 [Cargo.toml](src-tauri/Cargo.toml) |
| C++ 工具链 | VS 2022 Build Tools 的“使用 C++ 的桌面开发”和 Windows SDK |
| .NET | .NET 8 SDK |
| Python | 可选，仅管理员启动脚本使用 |

在仓库根目录执行以下 PowerShell 命令：

```powershell
npm ci
npm run tauri:dev
```

`tauri:dev` 在 helper 缺失或源码更新时编译它，再启动 Vite 和桌面程序。默认开发地址为 `http://localhost:5173`。需要完整硬件能力时，在管理员终端运行；也可以使用 `python run-dev-admin.py`，由脚本请求 UAC 提权。该脚本会结束它识别出的当前项目旧进程并重新启动开发环境。

仅调试前端时使用 `npm run dev`；普通浏览器的模拟 IPC 入口见[隔离界面验收](tests/ui/README.md)。

## 构建与打包

```powershell
# 前端类型检查和生产构建
npm run build

# 强制重新发布硬件 helper
npm run build:helper

# 发布 helper、构建前端和 Rust，并生成 NSIS 安装包
npm run tauri:build
```

正式更新包需要设置 `TAURI_SIGNING_PRIVATE_KEY`，并与 [tauri.conf.json](src-tauri/tauri.conf.json) 中的更新公钥匹配。仅验证桌面编译、不生成安装包时使用 `npm run tauri:build -- --no-bundle`。

默认安装包位于 `target/release/bundle/nsis/`；指定 Rust 目标时位于 `target/<目标>/release/bundle/nsis/`。Release 默认启用 LTO；可通过 `CARGO_PROFILE_RELEASE_LTO` 等 Cargo 环境变量覆盖。

也可运行 `powershell -ExecutionPolicy Bypass -File build.ps1`，使用交互菜单选择 LTO / 普通构建或清理构建缓存，并将安装包复制到 `release/`。脚本沿用 Tauri 的前端构建钩子，前端只编译一次。菜单中的版本输入会修改版本文件；需要变更版本时还应同步锁文件。

[GitHub Actions](.github/workflows/build.yml) 构建 x64 和 ARM64：推送 `v*` 标签时发布安装包、签名及 `latest.json`，手动触发时上传构建产物。helper 的架构必须与桌面目标一致，例如 ARM64 使用：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Runtime win-arm64
npx tauri build --target aarch64-pc-windows-msvc --bundles nsis
```

## 测试与检查

全新检出后，先运行 `npm ci`、`npm run build:helper` 和 `npm run build`，准备依赖、打包资源和前端输出。

```powershell
# 前端行为回归、类型检查和构建
npm test
npm run build
npx --no-install tsc -p tests/ui/tsconfig.json

# Rust 单元测试与严格静态检查
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --all --check

# 硬件合成回归，不启动真实采集程序
# artifacts-path 隔离测试输出，避免与开发中的 helper 冲突
dotnet build tests/hw-helper/HwHelper.Tests.csproj --artifacts-path target/hw-validation -p:SelfContained=false
dotnet target/hw-validation/bin/HwHelper.Tests/debug/HwHelper.Tests.dll
```

前端测试覆盖状态恢复、请求去重、清理任务跨页、会话导航、虚拟列表、风扇选择和读数显示；Rust 回归覆盖文件删除边界、会话存储、OAuth、配置与采样等逻辑。涉及删除的回归使用合成临时目录。硬件测试通过虚构设备和内存传感器验证采集逻辑，不查询 WMI、不加载驱动、不操作真实风扇。

[隔离界面验收](tests/ui/README.md) 使用真实组件与模拟 IPC。真实桌面 IPC、主板与驱动差异，以及同负载下的 CPU、内存和帧率表现需要单独验收。

## 项目结构

```text
SysPulse/
├── .cargo/config.toml      统一 Cargo 输出到根目录 target/
├── .github/workflows/      x64 / ARM64 构建与发布
├── index.html              配置窗口入口
├── src/
│   ├── routes/             页面和路由注册
│   ├── components/         组件与交互逻辑
│   ├── stores/             应用级状态、采集和后台清理任务
│   ├── utils/              公共工具；回归测试与源码就近放置
│   ├── ipc.ts              IPC 包装
│   └── bindings.ts         Rust 自动生成的类型与命令
├── overlay/                独立悬浮窗入口，纯 TypeScript / CSS
├── src-tauri/
│   ├── src/ai/             MCP、OAuth 和多来源会话管理
│   ├── src/hw/             helper 通信、硬件采样和风扇保护
│   ├── src/monitor/        CPU、内存和网络采集
│   ├── src/ipc/            命令注册、清理服务和事件
│   ├── src/config/         配置读取、规范化和通知
│   ├── src/storage/        SQLite 流量持久化
│   ├── src/windows_api/    Win32 接口封装
│   ├── icons/              打包图标和源图
│   └── tauri.conf.json     窗口、资源、安装包与更新配置
├── hw-helper/              .NET 硬件采集和控制程序
├── scripts/                helper 构建与按需更新
├── public/                 侧栏和 favicon 共用图标
├── tests/hw-helper/         C# 合成回归
├── tests/ui/               浏览器模拟 IPC 入口与验收说明
├── build.ps1               交互式构建、缓存清理和安装包复制
└── run-dev-admin.py         可选的管理员开发启动入口
```

```mermaid
flowchart LR
    UI[配置窗口 / 悬浮窗] <-->|IPC 与事件| Rust[Rust 应用服务]
    Rust --> Basic[CPU / 内存 / 网络采集]
    Rust <-->|逐行 JSON| Helper[.NET helper / LibreHardwareMonitor]
    Rust --> Cleanup[后台扫描与清理]
    Rust --> AI[MCP / OAuth / 会话管理]
    Rust --> Storage[配置 / SQLite / 日志]
```

基础数据与硬件数据使用独立采样循环。硬件读取完成后再等待下一轮，失败时退避；前端从事件与共享状态获取进度和快照。

## 维护约定

Rust 库只公开应用启动与绑定导出入口。IPC 的生产注册和测试处理器共用 [命令清单](src-tauri/src/ipc/commands/mod.rs)。修改命令或数据类型后，在根目录重新生成绑定：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --locked --bin export_bindings
```

不要手工修改 `src/bindings.ts`。`package-lock.json` 与 `src-tauri/Cargo.lock` 保留在版本控制中；Tauri 版本读取 `package.json`，发布前同步 Cargo 包版本及相关锁文件。

`node_modules/`、`dist/`、`target/`、`src-tauri/gen/`、`hw-helper/bin/`、`obj/`、`publish/` 和 `src-tauri/resources/hw-helper/` 都是生成物。后者是桌面运行和打包资源，不应在程序运行时删除。TypeScript 增量信息放在 `node_modules/.cache/`；测试构建产物可用 `dotnet clean` 或指定 `--target-dir` 的 `cargo clean` 清理。

硬件库固定为 LibreHardwareMonitorLib **0.9.4**，升级时需复核两个适配点：

- `SnapshotBuilder.CorrectStorageRate` 修正该版本将 `Stopwatch` 计数按 `TimeSpan` ticks 换算的存储速率问题，避免升级后重复修正。
- `FanReadingFreshness` 在 NVIDIA 更新前清空精确 Sensor 类型的内存缓存，避免驱动读取失败后沿用旧值。该操作不写硬件控制；类型或属性不匹配时返回未知。`FanChannels.Resolve` 则由快照和控制写入共用，拒绝按相同索引猜测 RPM/PWM 关联。

上游依据：[存储采样](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.4/LibreHardwareMonitorLib/Hardware/Storage/AbstractStorage.cs)、[Sensor.Value](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.4/LibreHardwareMonitorLib/Hardware/Sensor.cs)、[NVIDIA 更新](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.4/LibreHardwareMonitorLib/Hardware/Gpu/NvidiaGpu.cs)、[主板通道标签](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.4/LibreHardwareMonitorLib/Hardware/Motherboard/SuperIOHardware.cs)。

## 数据位置与排查

| 内容 | 默认位置 |
| --- | --- |
| 配置 | `%APPDATA%\SysPulse\config.toml` |
| 流量数据库 | `%APPDATA%\SysPulse\traffic.db` |
| 日志 | `%LOCALAPPDATA%\SysPulse\logs\app.log.YYYY-MM-DD` |
| AI 管理数据 | `%LOCALAPPDATA%\SysPulse\ai-tools\` |
| 清理热点索引 | `%LOCALAPPDATA%\SysPulse\cleanup-hotspots-v1.json` |

提高开发日志级别：

```powershell
$env:RUST_LOG = 'debug,wry=info,tao=info'
npm run tauri:dev
```

通用页配置读取失败时，可展开并复制首次失败诊断，再点击“重新读取”。重试成功不会抹除首次诊断；历史上首次 IPC 失败的具体根因仍需真实错误信息复现。当前生产构建有主入口体积提示，通用页有 Ant Design `InputNumber.addonAfter` 弃用提示。

## 第三方与许可

项目自有代码采用 MIT。AI 管理后端部分代码源自 **KiroHub `637321c`**，保留 **CC-BY-NC-SA-4.0** 授权，全文见 [LICENSE.KiroHub](src-tauri/src/ai/backend/LICENSE.KiroHub)。该后端直接参与本项目 Rust 编译，适配层位于 `src-tauri/src/ai/` 和 `src-tauri/src/ipc/commands/ai_cmd.rs`。

硬件传感器使用 **LibreHardwareMonitorLib（MPL 2.0）**。第三方组件分别遵循其许可证。
