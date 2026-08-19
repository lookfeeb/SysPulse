# AI 管理后端

此目录是 SysPulse 后端的一部分，直接参与 `syspulse` crate 编译，不是独立 crate 或外部路径依赖。

包含：

- Kiro、Codex、Claude CLI 的 MCP 配置读取、扫描、复制、启停和删除。
- 远程 MCP OAuth、本地认证代理、Windows DPAPI 凭据加密与 Token 自动刷新。
- Kiro IDE/CLI、Codex、Claude、Antigravity、Gemini 会话扫描、解析、导出和删除；Codex 同时覆盖活动/归档正文、旧 SQLite 目录、索引和历史备份。
- AI 管理专用 SQLite 与原子文件写入支持。

核心代码迁移自 KiroHub `637321c`，原项目许可证为 `CC-BY-NC-SA-4.0`，许可证全文见
`LICENSE.KiroHub`。SysPulse 适配层位于上一级 `mod.rs`、`models.rs` 和
`src-tauri/src/ipc/commands/ai_cmd.rs`。
