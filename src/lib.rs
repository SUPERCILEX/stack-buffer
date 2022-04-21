#![feature(maybe_uninit_slice)]
#![feature(read_buf)]
#![feature(can_vector)]
#![feature(maybe_uninit_uninit_array)]

mod bufreader;
mod bufwriter;

pub use bufreader::StackBufReader;
pub use bufwriter::StackBufWriter;
