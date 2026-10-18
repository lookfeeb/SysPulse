# 隔离界面验收

本入口使用 Tauri 的 `mockIPC` / `mockWindows` 加载真实组件；未知命令报错，模拟扫描、删除和 OAuth 只改变内存数据，刷新后恢复。它不进入生产构建，也不代替真实桌面、文件系统、硬件或 OAuth 服务验收。

在仓库根运行：

```powershell
node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 5175
```

打开 `http://127.0.0.1:5175/tests/ui/index.html`。类型检查命令为 `npx --no-install tsc -p tests/ui/tsconfig.json`；其他检查见[项目 README](../../README.md)。

## 场景

| 地址参数 | 数据与检查重点 |
| --- | --- |
| 无参数 | 一万条会话、80 条消息分页、12 个概览采样点、模拟清理和 OAuth；检查虚拟列表、键盘导航、详情分页、删除确认及授权状态 |
| `?session-sidebar` | 同名不同路径项目、空项目、只读备份、链接前缀与长标题；检查项目搜索、完整标题检索、来源过滤、页码恢复和慢请求切换 |
| `?config-failure` | 配置请求先失败后恢复；检查错误提示、首次失败诊断和“重新读取” |
| `?cleanup-background` | 扫描和清理等待显式完成；检查切页后进度延续、报告保留和取消 |

在 1000×680 和 800×560 视口检查页面与弹窗无横向溢出，查看控制台错误。通用页现有 `InputNumber.addonAfter` 弃用提示见项目 README。

## 会话控制

浏览器控制台加载：

```javascript
const { sessionFixture } = await import('/tests/ui/fixtures.ts');
```

`listDelayMs`、`loadDelayMs` 控制延迟，`failNextList`、`failNextLoad` 注入一次失败；`listCalls`、`loadCalls`、`deleteCalls`、`remaining` 用于核对请求和数据。

检查慢加载期间切换项目不会被旧响应覆盖；删除取消不改数据，重复确认只提交一次；删除成功但刷新失败时，已删除行仍应消失。只读记录保持只读。搜索“尾部检索目标”应匹配超过显示长度的标题；列表快捷键包括 ↓、End、Enter 和 Ctrl+F。

## 后台清理控制

在 `?cleanup-background` 场景加载：

```javascript
const { cleanupFixture } = await import('/tests/ui/fixtures.ts');
```

1. 点击开始扫描，调用 `await cleanupFixture.progress(123)`；切到概览，再调用 `await cleanupFixture.progress(456)`。返回清理页仍为扫描中，`scanCalls` 为 1。
2. 调用 `cleanupFixture.finishScan()`；确认模拟清理后切页，`cleanCalls` 为 1。
3. 调用 `cleanupFixture.finishClean()`，进入后台刷新；返回清理页显示释放 512 B、跳过锁定文件的紧凑报告。
4. 再调用 `cleanupFixture.finishScan()`，刷新完成且报告保留；展开说明检查路径换行与滚动。`finishScan(true)` 可模拟取消结果。

## 硬件控制

```javascript
const { hardwareFixture } = await import('/tests/ui/fixtures.ts');
await hardwareFixture.publish();
```

CPU 温度应为 62°C，缺失占用显示未知；等待 6 秒后旧读数清空，再次发布后恢复。

`publish(fans)` 接受模拟 `FanHw[]`；`fanWrites` 记录请求，`fanWriteDelayMs` 控制延迟，`failNextFanWrite` 注入失败。检查只有唯一明确 CPU 风扇时才默认选择、0/未知/高速读数可区分、控制关联未知时只读、传感器消失不自动替换，以及重复操作去重和失败回退。所有写入仅修改模拟状态。
