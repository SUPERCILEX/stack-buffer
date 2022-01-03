#![feature(maybe_uninit_slice)]
#![feature(read_buf)]
#![feature(can_vector)]

mod bufreader;

pub use bufreader::StackBufReader;
