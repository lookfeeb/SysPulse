// Bin entry point — keeps the Windows subsystem flag for release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    syspulse_lib::run();
}
