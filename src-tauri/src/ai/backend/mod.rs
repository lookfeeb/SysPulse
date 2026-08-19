//! SysPulse 内置的 AI 管理后端。
//!
//! 负责 MCP 配置、OAuth、本地代理、Token 刷新和多来源会话解析。

pub mod commands;
pub mod db;
pub mod kiro;
pub mod mcp_oauth;
pub mod mcp_proxy;
pub mod models;
pub mod oauth_store;
pub mod runtime;
pub mod services;
pub mod tasks;
pub mod utils;
