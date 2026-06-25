// main.rs - エントリポイント。Windows ではコンソール窓を出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    picoagent_lib::run();
}
