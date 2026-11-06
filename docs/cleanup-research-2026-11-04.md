# Windows 系统垃圾清理源码研究

> 历史研究资料：保留外部源码链接、许可边界和设计依据。下文对 SysPulse 的描述对应研究日期，部分建议已在后续重构中实现；当前调用关系与实现入口见[架构文档](architecture.md)。

研究日期：2026-11-04。范围：GitHub 公开代码、微软官方文档，以及 SysPulse 现有清理模块的针对性对照。

本次只做源码阅读和研究，没有运行下载的清理脚本，没有清理本机文件，没有修改程序实现。下列结论来自固定提交的静态分析，不代表这些项目已经通过本机运行验证，也不是对 SysPulse 的完整安全审计。

## 1. 结论

最值得参考的组合是：BleachBit 的规则与执行器分离、Winapp2 的应用路径资料、微软提供的系统维护入口，以及 Czkawka 的分阶段文件扫描算法。PowerShell 清理脚本适合研究流程和反例，不适合直接作为桌面产品的删除后端。

SysPulse 已经有多配置目录发现、风险分级、保留期、重解析点检查、根目录身份复核、变化项跳过、取消及扫描并发限制。后续优先补充规则证据、应用版本识别和 Windows Update 专用处理，而非单纯增加递归删除路径。

## 2. 研究对象与固定版本

| 项目 | 阅读内容 | 值得借鉴 | 使用边界 |
| --- | --- | --- | --- |
| [BleachBit](https://github.com/bleachbit/bleachbit/tree/51a17163958276bf5d49f688c413eeecb3eb8d5d) | Chromium、Firefox XML；CleanerML.py；FileUtilities.py | 声明式规则、进程识别、按数据语义拆分选项、限制不可信规则动作 | GPL-3.0；缓存、历史、密码、会话是不同功能，不能整份照搬 |
| [Winapp2](https://github.com/MoscaDotTo/Winapp2/tree/53ae41946c3d3c8bb348d81081c34b6050f0b561) | Winapp2.ini、README、License | 应用检测、文件匹配、排除规则；补充路径覆盖 | CC BY-SA 4.0；混合了缓存、日志、隐私和注册表清理规则 |
| [Remote-Disk-Cleanup](https://github.com/Myraas/Remote-Disk-Cleanup/tree/a297970509d25cb83859e805f5c5fc7c808e1549) | 主 PowerShell 脚本、README | 清理日志、回收站保留期、流程组织 | GPL-3.0；默认目标包含 Cookies 和 Documents 下的 `*tmp` |
| [ComputerCleanup](https://github.com/tomskovich/ComputerCleanup/tree/c4fcfc5ffd0d4f00654d905c0cf04831b530e3e9) | 浏览器、系统文件、更新下载、CleanMgr 等模块 | 按功能拆分、清理前后报告 | GitHub API 未识别许可证；复用前需进一步确认授权。存在强制结束浏览器等做法 |
| [Czkawka](https://github.com/qarmin/czkawka/tree/1453a6b94adca70af5ba1ecb60d0da1dee0680f9) | duplicate/core.rs、mod.rs、核心许可证 | 大小分组、预哈希、完整哈希、缓存和硬链接过滤 | 所读 czkawka_core 为 MIT；其他应用及素材许可证不同。重复文件不等于垃圾 |

许可证说明用于规划代码或规则复用；自行实现思路与直接复制代码、规则数据是不同情形。

## 3. 源码中的关键发现

### BleachBit：学习规则模型与数据分类

[chromium.xml](https://github.com/bleachbit/bleachbit/blob/51a17163958276bf5d49f688c413eeecb3eb8d5d/cleaners/chromium.xml) 分别定义 `cache`、`cookies`、`history`、`session`、`site_data`、`passwords` 等选项。`site_data` 明确包含 Local Storage 和 IndexedDB；这些目录可能保存网站离线数据，不能归入普通缓存默认删除。

同一个规则文件还声明运行中的应用。其意义是应用状态应该参与清理决策，不能只根据目录存在就执行删除。

[CleanerML.py](https://github.com/bleachbit/bleachbit/blob/51a17163958276bf5d49f688c413eeecb3eb8d5d/bleachbit/CleanerML.py) 将规则解析成动作，并对不可信规则中的部分命令进行拦截。SysPulse 若以后开放用户规则，应采用受限动作集合和根目录约束，不能让下载的规则任意执行 PowerShell、命令行或注册表写入。

[FileUtilities.py](https://github.com/bleachbit/bleachbit/blob/51a17163958276bf5d49f688c413eeecb3eb8d5d/bleachbit/FileUtilities.py) 包含符号链接、Windows junction、重解析标签及删除相关处理。借鉴时应审查完整调用路径，不能把单个 `is_symlink` 判断视为全部删除安全保障。

### Winapp2：适合作为线索库，不能直接当安全白名单

[Winapp2.ini](https://github.com/MoscaDotTo/Winapp2/blob/53ae41946c3d3c8bb348d81081c34b6050f0b561/Winapp2.ini) 使用 `Detect` / `DetectFile` 判断应用，`FileKey` 声明目标，`ExcludeKey` 排除应保留的数据。

实际规则表明，“应用清理”远超“缓存清理”：Discord 条目包含 `blob_storage`、`QuotaManager` 等；Teams 条目涉及 History、Web Data、客户端证书相关目录等。不能仅凭规则名称把全部动作导入 SysPulse。

可进一步核实的覆盖线索包括 NVIDIA `LocalLow\NVIDIA\DXCache`、`LocalLow\NVIDIA\PerDriverVersion\DXCache`，以及不同版本 Teams 的数据根。它们仍需应用版本、真实目录样本和占用行为验证；不建议照搬 `*Cache*` 之类宽泛匹配。

### PowerShell 脚本：四类具体问题

1. **目录年龄不能代替内部文件年龄。** [Optimize-SystemFiles.ps1 第 58–60 行](https://github.com/tomskovich/ComputerCleanup/blob/c4fcfc5ffd0d4f00654d905c0cf04831b530e3e9/Public/Optimize-SystemFiles.ps1#L58-L60) 枚举文件和目录，按时间筛选后递归删除。旧目录内部仍可能有新近修改的文件，因此会绕过逐文件保留期。此外，表达式中的 `CreationTime` 只被当成真假值，没有与截止日期比较。
2. **强制关闭应用会影响用户工作。** [Clear-BrowserCache.ps1 第 108 行](https://github.com/tomskovich/ComputerCleanup/blob/c4fcfc5ffd0d4f00654d905c0cf04831b530e3e9/Public/Clear-BrowserCache.ps1#L108) 使用 `Stop-Process -Force`；目标列表还包含 DOMStore 和 Firefox 站点存储相关文件，不能视为纯磁盘缓存。
3. **清理范围可能跨入用户数据。** [Remote-Disk-Cleanup.ps1](https://github.com/Myraas/Remote-Disk-Cleanup/blob/a297970509d25cb83859e805f5c5fc7c808e1549/Remote-Disk-Cleanup.ps1) 的默认调用包括 `Documents\*tmp` 和 Cookies；其通用 `Clean-Path` 直接递归删除，回收站的保留期逻辑没有覆盖这条通用路径。
4. **静默错误会让服务维护流程失真。** [Clear-SoftwareDistribution.ps1](https://github.com/tomskovich/ComputerCleanup/blob/c4fcfc5ffd0d4f00654d905c0cf04831b530e3e9/Public/Clear-SoftwareDistribution.ps1) 停止服务、改名、删除、再启动；改名和删除使用 `SilentlyContinue`，没有用 `finally` 恢复原始服务状态。服务停止失败也未明确阻断后续步骤，不能作为可靠的产品实现。

这些是所读代码的静态结论，没有实际运行上述操作来复现。

### Czkawka：适合将来的重复文件分析

[duplicate/core.rs](https://github.com/qarmin/czkawka/blob/1453a6b94adca70af5ba1ecb60d0da1dee0680f9/czkawka_core/src/tools/duplicate/core.rs) 的哈希查重路径先按大小筛选，再进行预哈希，最后计算完整哈希，并提供缓存与硬链接过滤。这可以显著减少需要完整读取的文件量。

如果 SysPulse 增加重复文件功能，应独立于一键清理，结果默认不选中，明确保留副本；不能把文件名、大小或预哈希相同视为可自动删除的依据。内容相同也不意味着其中一份不再被程序或用户需要。

## 4. 微软文档确定的系统边界

| 对象 | 研究结论 | 来源 |
| --- | --- | --- |
| WinSxS | 不能直接删除目录或内部文件；使用受支持的组件维护机制 | [Clean Up the WinSxS Folder](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/clean-up-the-winsxs-folder) |
| DISM `/StartComponentCleanup` | 可作为独立的高级系统维护操作；与计划任务自动维护的等待及超时行为有区别 | 同上 |
| DISM `/ResetBase` | 完成后，现有更新包无法卸载；不适合作为日常垃圾清理默认参数 | 同上 |
| Windows Installer 缓存 | `%SystemRoot%\Installer` 不应被删除，否则应用修复、升级、卸载可能失败 | [Missing Windows Installer cache](https://learn.microsoft.com/en-us/troubleshoot/windows-client/application-management/missing-windows-installer-cache) |
| `cleanmgr` | `/sageset:n` 保存选择，`/sagerun:n` 使用该配置；后者枚举所有驱动器，`/d` 不限制其范围 | [cleanmgr](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cleanmgr) |
| Delivery Optimization | 存在专用 `Delete-DeliveryOptimizationCache` 命令；应验证系统版本、模块和参数后单独接入 | [官方命令页](https://learn.microsoft.com/en-us/powershell/module/deliveryoptimization/delete-deliveryoptimizationcache) |

Delivery Optimization 文档部分描述仍是占位文本，本次只确认命令入口及参数存在，不据此承诺各版本的清理效果或安全性。

## 5. 对 SysPulse 的具体建议

### 优先级 P0：明确系统维护与文件清理的边界

现有 `scan.rs` 将 `SoftwareDistribution\Download` 列为 `win-update`，默认属于 Advanced 且不勾选；`clean.rs` 中仍进入通用目录清理。所读清理模块没有发现对应的 Windows Update 服务状态和更新任务专用处理。

建议把该类别转为独立维护流程，补充更新状态检测、明确的失败处理和结果报告；在完成前可保持仅扫描展示。这比新增清理目录更优先。此处是静态架构发现，尚未通过运行中的更新场景验证。

### 优先级 P1：把已有规则变得可维护、可解释

建议每条规则至少记录：`id`、来源 URL/提交、适用应用与版本、已确认根目录、包含/排除模式、保留期、风险、默认选择、进程条件、执行方式和效果说明。

继续使用现有 Rust 执行后端。普通规则只允许受限的文件操作；系统维护走单独实现。为每个新增规则准备合成目录样本，证明缓存被命中、用户数据被保留，而不是仅测试“目录能被删除”。

### 优先级 P1：补充应用发现和进程判定

- 浏览器已经支持多个配置目录；可核实 BleachBit 提到的 `GraphiteDawnCache`、`GPUPersistentCache` 等新路径，避免对整个 User Data 或站点存储下手。
- Teams 应区分旧版 Electron 与新版打包应用，验证数据根后添加精确缓存子目录。
- NVIDIA 可核实 LocalLow 下的缓存变体；着色器缓存应保持可选，并说明清理后首次运行可能重新编译。
- 当前 `remove.rs` 的进程阻断依赖命令行/工作目录与目标路径关联，普通浏览器进程未必暴露缓存路径；监控名单也没有 Vivaldi。建议补充同一用户的进程识别、显式配置目录参数和无法确定占用时的保守处理，不应宣称现有进程检查一定能识别所有浏览器占用。
- 系统路径应从 Known Folder / 系统环境解析，减少 `C:\Windows`、`C:\ProgramData` 等硬编码对非默认安装的遗漏。

### 优先级 P2：完善安全验证与结果口径

现有实现会复核根目录身份、逐项检查重解析点及修改时间，并优先使用分配大小统计，这些值得保留。但 `remove.rs` 最终仍按路径调用 `std::fs::remove_file`，检查和删除之间存在时间窗口；不能把现有检查等同于已经消除所有 TOCTOU 竞争。管理员清理用户可写目录时，后续应专门验证祖先目录替换、junction 竞争、硬链接及云文件场景，必要时采用句柄级约束。

结果展示应区分扫描估算、成功删除项的统计、跳过项及磁盘可用空间变化。硬链接、压缩、延迟删除和后台写入都会使这些值不同，不能把整盘前后空间差全部归因于本次清理。

## 6. 后续实现的验收场景

- 旧目录里放一个新文件：只删除符合保留期的旧文件。
- 缓存目录旁放 Cookies、Local Storage、IndexedDB、会话及项目源文件：全部保留。
- 扫描后新增/修改文件，替换根目录或插入 junction：变化项跳过，不能越界删除。
- 应用运行中、文件占用、权限不足：输出准确的跳过原因，不强制结束应用。
- Windows Update 正在下载或安装：维护操作拒绝或延后执行，不进入通用递归删除。
- 多用户、多浏览器配置、非 C 盘系统：识别正确范围，不扩大到未选择用户。
- 新增规则版本更新：保留来源、差异和回归样本，未经验证的规则不进入默认勾选集合。

建议实施顺序：Windows Update 边界 → 规则元数据和回归样本 → 应用版本/路径覆盖 → 删除竞争专项验证 → 可选的重复文件分析。
