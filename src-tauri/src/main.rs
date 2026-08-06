// main.rs - エントリポイント。Windows ではコンソール窓を出さない。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    picoagent_lib::run();
}
