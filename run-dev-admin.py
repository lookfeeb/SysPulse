"""双击运行：以管理员身份打开 PowerShell 7 并启动 `npm run tauri:dev`。"""

import ctypes
import base64
import json
import os
import shutil
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


def powershell7_path() -> str:
    candidates = [
        os.path.join(os.environ.get("ProgramW6432", os.environ.get("ProgramFiles", "C:\\Program Files")), "PowerShell", "7", "pwsh.exe"),
        shutil.which("pwsh.exe"),
    ]
    for candidate in dict.fromkeys(path for path in candidates if path):
        if not os.path.isfile(candidate):
            continue
        try:
            version = subprocess.run(
                [candidate, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$PSVersionTable.PSVersion.Major"],
                capture_output=True, text=True, check=True, timeout=15,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
        except (OSError, subprocess.SubprocessError):
            continue
        if version.stdout.strip() == "7":
            return os.path.abspath(candidate)
    raise RuntimeError("未找到可用的 PowerShell 7。请安装 PowerShell 7（pwsh.exe）后重试；不会使用 Windows PowerShell 5。")


def cargo_target_dir() -> str:
    env = os.environ.copy()
    env.pop("CARGO_TARGET_DIR", None)
    metadata = subprocess.run(
        ["cargo", "metadata", "--manifest-path", "src-tauri/Cargo.toml", "--no-deps", "--format-version", "1", "--locked"],
        cwd=PROJECT_DIR, env=env, capture_output=True, text=True, encoding="utf-8", check=True,
    )
    # Resolve mapped drives before elevation: the elevated token may not have Z:.
    return os.path.realpath(json.loads(metadata.stdout)["target_directory"])


def dev_command(pwsh_path: str, target_path: str) -> str:
    project_dir = ps_quote(PROJECT_DIR)
    target_dir = ps_quote(target_path)
    # 透传当前用户的 PATH（含 node/npm/cargo/rustup 等），避免提权后 Administrator 账户找不到
    current_path = ps_quote(os.path.dirname(pwsh_path) + os.pathsep + os.environ.get("PATH", ""))
    return "; ".join(
        [
            f"$env:Path = {current_path} + ';' + $env:Path",
            f"Set-Location -LiteralPath {project_dir}",
            f"$env:CARGO_TARGET_DIR = {target_dir}",
            "$targetPrefix = $env:CARGO_TARGET_DIR.TrimEnd('\\') + '\\'",
            "$project = (Get-Location).Path",
            (
                "function Test-ProjectProcess([int]$processId) { "
                "$proc = Get-CimInstance Win32_Process -Filter \"ProcessId=$processId\" -ErrorAction SilentlyContinue; "
                "if (-not $proc) { return $false }; "
                "$path = $proc.ExecutablePath; "
                "$cmd = $proc.CommandLine; "
                "if ($path -and $path.StartsWith($project, [System.StringComparison]::OrdinalIgnoreCase)) { return $true }; "
                "if ($path -and $path.StartsWith($targetPrefix, [System.StringComparison]::OrdinalIgnoreCase)) { return $true }; "
                "if ($cmd -and $cmd.IndexOf($project, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) { return $true }; "
                "return $false "
                "}"
            ),
            (
                "Get-Process syspulse -ErrorAction SilentlyContinue | "
                "Where-Object { Test-ProjectProcess $_.Id } | "
                "Stop-Process -Force -ErrorAction SilentlyContinue "
            ),
            (
                "$ports = @(6541, 6542); "
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


def main() -> None:
    if sys.platform != "win32":
        print("此脚本仅支持 Windows。")
        sys.exit(1)

    try:
        pwsh_path = powershell7_path()
        ps_command = dev_command(pwsh_path, cargo_target_dir())
    except (RuntimeError, OSError, subprocess.SubprocessError, ValueError, KeyError) as error:
        print(f"启动失败：{error}")
        input("按回车键退出...")
        sys.exit(1)

    encoded_command = base64.b64encode(ps_command.encode("utf-16le")).decode("ascii")
    arguments = ["-NoLogo", "-NoProfile", "-NoExit", "-ExecutionPolicy", "Bypass", "-EncodedCommand", encoded_command]

    if is_admin():
        subprocess.run(
            [pwsh_path, *arguments],
            cwd=PROJECT_DIR,
        )
        return

    params = subprocess.list2cmdline(arguments)
    rc = ctypes.windll.shell32.ShellExecuteW(
        None,           # hwnd
        "runas",        # 触发 UAC 提权
        pwsh_path,
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
