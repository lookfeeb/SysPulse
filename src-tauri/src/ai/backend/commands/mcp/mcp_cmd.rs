#[path = "mcp_cmd/adapters.rs"]
mod adapters;
#[path = "mcp_cmd/client_commands.rs"]
mod client_commands;
#[path = "mcp_cmd/discovery.rs"]
mod discovery;
#[path = "mcp_cmd/legacy_kiro.rs"]
mod legacy_kiro;
#[path = "mcp_cmd/types.rs"]
pub mod types;

pub(crate) use adapters::{load_mcp_items_for_client, McpClientKind};
pub use adapters::{
    read_mcp_server_url_for_client, write_mcp_server_url_for_client,
    write_mcp_server_url_for_client_if_current,
};
pub use client_commands::*;
pub use discovery::*;
pub use legacy_kiro::*;
pub use types::{McpClientStats, McpClientsOverview, McpServerItem};
