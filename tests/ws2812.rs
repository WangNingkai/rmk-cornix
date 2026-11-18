// 直接编译实际状态模块，避免宿主测试引入 nRF 外设和链接脚本。
#[path = "../src/ws2812/state.rs"]
mod state;
