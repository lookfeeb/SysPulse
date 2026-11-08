# hw-helper 验证与性能测量

`Program.cs` 使用合成设备和 LHM 内存传感器验证读数、失败恢复、风扇通道关联、慢磁盘隔离、关闭顺序、内存字节计数和 JSON 行协议，不打开驱动或执行真实风扇写入。

```powershell
dotnet run --project tests/hw-helper/HwHelper.Tests.csproj -c Release
```

## 当前生产链路：同进程 ABI

设计、数据来源和实测限制见 [同进程说明](../../docs/hardware-in-process.md)。回归覆盖 SMBIOS、USB/PCIe 能力与当前速率区分、USB BOS/SSP 属性解析、双向伴随端口合并、缺失协议/映射不推算上限，以及二进制 Unicode/null/零值/64 位计数和 SPW1/SPW2 跨语言兼容。

~~~powershell
npm run build:helper
$env:HW_ABI_FIXTURE = Join-Path $PWD 'target/hw-inprocess-build/abi-fixture.bin'
dotnet run --project tests/hw-helper/HwHelper.Tests.csproj -c Release
cargo test --manifest-path src-tauri/Cargo.toml --lib hw::
$env:HW_NATIVE_DIR = Join-Path $PWD 'src-tauri/resources/hw-helper'
$env:HW_NATIVE_JSON = Join-Path $PWD 'target/hw-inprocess-build/native-snapshot-final.json'
cargo test --manifest-path src-tauri/Cargo.toml --lib hw::client::tests::real_worker_snapshot_and_shutdown -- --ignored --nocapture --test-threads=1
~~~

实机测试仅初始化、读取和关闭，不设置风扇。另可运行 hw::native::tests::real_in_process_lifecycle 验证版本拒绝及重复启停。不要并行运行真实硬件测试。开发实例锁住输出 DLL 时先关闭实例；只验证测试/类型导出时可在该命令的环境使用 TAURI_CONFIG='{"bundle":{"resources":[]}}' 跳过资源复制，不能用于最终打包。

以下 JSON 探针/基准保留用于旧版对照，不代表当前生产同进程调用路径。

## 合成微基准

```powershell
dotnet run --project tests/hw-helper/HwHelper.Tests.csproj -c Release -- --benchmark
```

基准使用相同的 108 个内存传感器，测量邮箱读取、完整快照读取、快照构建和独立诊断 JSON 写出路径。每项包含预热及 21 批测量，报告每批平均耗时的中位数、P95，以及当前线程每次操作的托管分配量。JSON 写入复用的 `MemoryStream`，排除管道和调度耗时；它不代表整个进程零分配。

比较版本时应使用同一 SDK、配置、设备数量及 JIT 设置。本次微基准统一使用进程环境变量 `DOTNET_TieredCompilation=0`，避免分层编译在批次间改变结果。纳秒结果来自批量计时后均摊；例如计时器为 10MHz 时，单个时间戳的分辨率是 100ns，不能将均摊结果解释为单次调用的纳秒级测量保证。

## 真实 helper 只读探针

```powershell
python tests/hw-helper/probe.py <独立发布目录>/hw-helper.exe --output-dir target/hw-validation/run --burst 500
```

探针只发送 `snapshot` / `shutdown`。默认观测 22 秒，前 5 秒作为预热，稳态 CPU 百分比按单个逻辑核心计算。记录延迟分位数、实际发布间隔、工作集、私有内存和 helper 文件 SHA-256；二进制在测量期间被修改会使测量失败。`--burst` 在预热后追加指定数量的串行请求；它测量缓存 IPC，不能提高硬件采样频率。

`baseline`、`load`、`recovery` 是探针阶段名；只有显式传入 `--stress` 才会启动有界 CPU / 磁盘负载。不要把未启用该参数的结果当作压力测试。比较前后版本时使用两个独立发布目录，避免开发启动脚本刷新 `src-tauri/resources/hw-helper/` 后混淆版本。

输出中的 `helper-summary.json` 保存汇总，`helper-probe.json` 保存样本，stderr 文件保存采集异常。首次 `ready` 后普通传感器可能尚未发布，年龄为 `long.MaxValue` 表示尚无数据，应与稳态年龄分开看待。

## 准确度与资源边界

- 内存总量与可用量来自同一次 `GlobalMemoryStatusEx`，使用原始字节计数计算占用率；大内存、按页对齐的计数不会因浮点 GB 中转发生量化损失。
- 传感器标识缓存使用弱引用，不缓存可变的名称、当前值或控制对象；传感器消失后不由缓存延长其生命周期。
- 普通采集目标周期为 1 秒、磁盘为 3 秒，计入采集时间并跳过错过的周期。Windows 调度、驱动和设备调用仍会产生抖动；不保证硬实时响应。
- 新鲜度按采集开始时刻计算；读取缓存不会让旧数据重新变新。失败、未知和真实零值继续区分。
- 缓存读取可以测得纳秒数量级；实际驱动调用、WMI、进程调度和 JSON 管道请求不能据此承诺纳秒响应。
- 托管分配下降不等于工作集必然下降。常驻内存还包括 CLR、JIT、WMI、LHM 和驱动库；需要分别记录，不能只报告有利指标。
- CPU/GPU 温度的物理准确度需要对应驱动权限及独立参考设备；合成回归只能验证映射、过滤和新鲜度逻辑。
