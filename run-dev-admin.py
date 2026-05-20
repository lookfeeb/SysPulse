"""双击运行：以管理员身份打开 PowerShell 并启动 `npm run tauri:dev`。"""

import ctypes
import base64
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
    clean_incremental = "--clean" in sys.argv[1:]
    ps_command = "; ".join(
        [
            f"$env:Path = {current_path} + ';' + $env:Path",
            f"Set-Location -LiteralPath {project_dir}",
            "Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue",
            "$project = (Get-Location).Path",
            (
                "function Test-ProjectProcess([int]$processId) { "
                "$proc = Get-CimInstance Win32_Process -Filter \"ProcessId=$processId\" -ErrorAction SilentlyContinue; "
                "if (-not $proc) { return $false }; "
                "$path = $proc.ExecutablePath; "
                "$cmd = $proc.CommandLine; "
                "if ($path -and $path.StartsWith($project, [System.StringComparison]::OrdinalIgnoreCase)) { return $true }; "
                "if ($cmd -and $cmd.IndexOf($project, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) { return $true }; "
                "return $false "
                "}"
            ),
            (
                "$names = @('syspulse', 'traffic-monitor'); "
                "foreach ($name in $names) { "
                "Get-Process $name -ErrorAction SilentlyContinue | "
                "Where-Object { Test-ProjectProcess $_.Id } | "
                "Stop-Process -Force -ErrorAction SilentlyContinue "
                "}"
            ),
            *(
                [
                    (
                        "$incremental = [System.IO.Path]::GetFullPath((Join-Path $project 'target\\debug\\incremental')); "
                        "if (Test-Path -LiteralPath $incremental) { "
                        "$prefix = $incremental.TrimEnd('\\') + '\\'; "
                        "Get-ChildItem -LiteralPath $incremental -Directory -ErrorAction SilentlyContinue | "
                        "Where-Object { $_.Name -like 'syspulse*' -and $_.FullName.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase) } | "
                        "Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "
                        "}"
                    )
                ]
                if clean_incremental
                else []
            ),
            (
                "$ports = @(5173, 5174); "
                "foreach ($port in $ports) { "
                "Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue | "
                "Select-Object -ExpandProperty OwningProcess -Unique | "
                "Where-Object { $_ -and $_ -ne $PID } | "
                "ForEach-Object { "
                "if (Test-ProjectProcess $_) { "
                "Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue "
                "} else { "
                "Write-Host \"端口 $port 被非当前项目进程占用，未自动结束 PID=$_\" -ForegroundColor Yellow "
                "} "
                "} "
                "}"
            ),
            "npm run tauri:dev",
        ]
    )

    encoded_command = base64.b64encode(ps_command.encode("utf-16le")).decode("ascii")

    if is_admin():
        subprocess.run(
            ["powershell", "-NoExit", "-ExecutionPolicy", "Bypass", "-EncodedCommand", encoded_command],
            cwd=PROJECT_DIR,
        )
        return

    params = f"-NoExit -ExecutionPolicy Bypass -EncodedCommand {encoded_command}"
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
