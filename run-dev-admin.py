"""双击运行：以管理员身份打开 PowerShell 并启动 `npm run tauri:dev`。"""

import ctypes
import os
import subprocess
import sys

PROJECT_DIR = os.path.dirname(os.path.abspath(__file__))


def ps_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def is_admin() -> bool:
    try:
        return ctypes.windll.shell32.IsUserAnAdmin() != 0
    except Exception:
        return False


def main() -> None:
    if sys.platform != "win32":
        print("此脚本仅支持 Windows。")
        sys.exit(1)

    project_dir = ps_quote(PROJECT_DIR)
    # 透传当前用户的 PATH（含 node/npm/cargo/rustup 等），避免提权后 Administrator 账户找不到
    current_path = ps_quote(os.environ.get("PATH", ""))
    ps_command = "; ".join(
        [
            f"$env:Path = {current_path} + ';' + $env:Path",
            f"Set-Location -LiteralPath {project_dir}",
            "Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue",
            "Get-Process traffic-monitor -ErrorAction SilentlyContinue | Stop-Process -Force",
            (
                "$ports = @(5173); "
                "foreach ($port in $ports) { "
                "Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue | "
                "Select-Object -ExpandProperty OwningProcess -Unique | "
                "Where-Object { $_ -and $_ -ne $PID } | "
                "ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue } "
                "}"
            ),
            "npm run tauri:dev",
        ]
    )

    if is_admin():
        subprocess.run(
            ["powershell", "-NoExit", "-ExecutionPolicy", "Bypass", "-Command", ps_command],
            cwd=PROJECT_DIR,
        )
        return

    params = f'-NoExit -ExecutionPolicy Bypass -Command "{ps_command}"'
    rc = ctypes.windll.shell32.ShellExecuteW(
        None,           # hwnd
        "runas",        # 触发 UAC 提权
        "powershell.exe",
        params,
        PROJECT_DIR,    # 工作目录
        1,              # SW_SHOWNORMAL
    )
    if rc <= 32:
        print(f"启动失败，ShellExecute 返回码：{rc}（用户可能取消了 UAC）。")
        input("按回车键退出...")
        sys.exit(1)


if __name__ == "__main__":
    main()
