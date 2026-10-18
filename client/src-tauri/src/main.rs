// No console window for the tray app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ems_solver_lib::main_with_args(std::env::args().skip(1).collect());
}
