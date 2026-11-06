"""启动器回归：不真正提权、启动应用或结束进程。"""

import base64
import importlib.util
import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock, patch


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("run_dev_admin", ROOT / "run-dev-admin.py")
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)

PWSH = r"C:\Program Files\PowerShell\7\pwsh.exe"
TARGET = r"\\192.168.4.1\win_data\SysPulseBuildCache\target"


@unittest.skipUnless(os.name == "nt", "Windows launcher")
class LauncherTests(unittest.TestCase):
    def test_requires_major_version_seven(self):
        for version in ("5", "8", "7"):
            with self.subTest(version=version), patch.object(launcher.os.path, "isfile", return_value=True), patch.object(
                launcher.shutil, "which", return_value=PWSH
            ), patch.object(launcher.subprocess, "run", return_value=Mock(stdout=version)) as run:
                if version == "7":
                    self.assertTrue(launcher.powershell7_path().endswith("pwsh.exe"))
                else:
                    with self.assertRaisesRegex(RuntimeError, "PowerShell 7"):
                        launcher.powershell7_path()
                for call in run.call_args_list:
                    self.assertEqual(Path(call.args[0][0]).name, "pwsh.exe")
                    self.assertIn("-NoProfile", call.args[0])

    def test_missing_pwsh_does_not_fall_back(self):
        with patch.object(launcher.os.path, "isfile", return_value=False), patch.object(
            launcher.shutil, "which", return_value=None
        ), patch.object(launcher.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "不会使用 Windows PowerShell 5"):
                launcher.powershell7_path()
            run.assert_not_called()

    def test_both_launch_paths_use_verified_pwsh(self):
        for elevated in (True, False):
            with self.subTest(elevated=elevated), patch.object(launcher, "powershell7_path", return_value=PWSH), patch.object(
                launcher, "cargo_target_dir", return_value=TARGET
            ), patch.object(launcher, "is_admin", return_value=elevated), patch.object(
                launcher.subprocess, "run"
            ) as run, patch.object(launcher.ctypes.windll.shell32, "ShellExecuteW", return_value=42) as execute:
                launcher.main()
                if elevated:
                    args = run.call_args.args[0]
                    self.assertEqual(args[0], PWSH)
                    encoded = args[-1]
                    execute.assert_not_called()
                else:
                    args = execute.call_args.args
                    self.assertEqual(args[1:3], ("runas", PWSH))
                    encoded = args[3].split()[-1]
                    run.assert_not_called()
                command = base64.b64decode(encoded).decode("utf-16le")
                self.assertIn("$ports = @(6541, 6542)", command)
                self.assertIn("$env:CARGO_TARGET_DIR = '" + TARGET + "'", command)
                self.assertIn(str(Path(PWSH).parent), command)

    def test_command_parses_with_quoted_paths(self):
        pwsh = launcher.powershell7_path()
        with patch.object(launcher, "PROJECT_DIR", r"C:\Dev\User's project"):
            command = launcher.dev_command(pwsh, TARGET)
        # Parse only: none of the generated process-management commands execute.
        parser = (
            "$source = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('"
            + base64.b64encode(command.encode("utf-16le")).decode("ascii")
            + "')); $tokens = $null; $parseErrors = $null; "
            "[void][System.Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$parseErrors); "
            "if ($parseErrors.Count) { $parseErrors | Out-String | Write-Error; exit 1 }"
        )
        result = subprocess.run([pwsh, "-NoProfile", "-NonInteractive", "-Command", parser], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
