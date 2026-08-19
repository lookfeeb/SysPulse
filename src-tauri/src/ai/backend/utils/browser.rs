/// 使用系统默认浏览器打开 MCP OAuth 授权页。
pub fn open_browser_keep_session(url: &str) -> Result<(), String> {
    open_with_default_browser(url)
}

fn open_with_default_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|error| format!("打开浏览器失败: {error}"))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        open::that(url).map_err(|error| format!("打开浏览器失败: {error}"))?;
    }

    Ok(())
}
