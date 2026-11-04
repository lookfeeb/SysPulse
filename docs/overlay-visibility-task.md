# 悬浮窗不可见回归（2026-11-05）

- [x] 复现：运行中的 Release 悬浮窗只显示任务栏底色；仅将该窗口置前，监控文字立即恢复。
- [x] 定位：移除 Explorer `SetParent` 后，任务栏与悬浮窗同属顶层置顶窗口；Tao 0.35.3 对重复 `set_always_on_top(true)` 不执行 Win32 层级更新，位置相同时停靠函数也提前返回。
- [x] 修复：在停靠位置提前返回之前检查真实 Z 序，仅当任务栏位于悬浮窗上方时调用 `SetWindowPos(HWND_TOPMOST, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE)`。保留独立窗口和透明背景。
- [x] 原生回归：任务栏模拟窗口覆盖后恢复、正常层级不重复抬升、不改变焦点/位置/大小、不覆盖位于上方的菜单模拟窗口。`cargo test --manifest-path src-tauri/Cargo.toml --lib z_order_tests --locked -- --nocapture`：1 passed；`cargo fmt -- --check`、`git diff --check` 通过。
- [x] 首版修复构建：`cargo build --release --manifest-path src-tauri/Cargo.toml --bin syspulse --features custom-protocol --locked` 成功，2026-11-05 00:14:51 启动；SHA256 `0270CF7AB931C93CDFCDBBB7367C947E6D3964A4CA5DD0F1E2D54208988E91CB`。
- [x] 首版桌面复验发现不足：用户确认点击托盘或任务栏其他图标后，悬浮窗会先消失再出现。两秒轮询只能恢复，无法满足持续稳定显示。
- [x] 增加任务栏/顶层窗口前台、显示、重排事件监听，在 UI 消息循环收到事件时立即恢复层级；两秒定时器仅作异常恢复后备。窗口销毁和退出时解除监听。
- [x] 合并启动与设置变更的窗口初始化路径，避免启动时遗漏事件监听；窗口重建也重新安装。启动日志必须出现 `overlay z-order event hooks installed` 才进入桌面复验。
- [x] 用真实 Win32 事件验证连续重排的恢复延迟与解除监听效果。最终针对性测试 2 passed；连续 30 次重排最大恢复延迟 5.1356 ms，未使用轮询定时器；菜单模拟窗口显示后仍在上方；解除监听后不再自动改层级。
- [x] 最终 Release 构建成功（2026-11-05 00:29:10），SHA256 `D761D310169865248A315199A6723D47E92A33889359C01A2F2162010188F749`。00:29:25 启动主进程 42664，00:29:27.412 日志确认 `overlay z-order event hooks installed`；硬件助手进程 33064。
- [x] 桌面复验通过：最终构建运行后，用户按要求连续点击托盘、任务栏其他图标并打开悬浮窗右键菜单，确认“显示稳定，菜单正常”。

本次仅修复窗口显示层级；已有重构记录见 `refactor-task.md`。
