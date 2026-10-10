// lib.rs - PicoAgent から独立して再利用できるエージェント中核。

#![forbid(unsafe_code)]

pub mod hash;
pub mod message;
pub mod paths;
pub mod permissions;
pub mod text;

pub mod whisper;
