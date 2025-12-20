pub mod backend;
pub mod stub;
pub mod x86_64_hosted;

pub use backend::CodegenBackend;
#[allow(unused_imports)]
pub use stub::StubBackend;
pub use x86_64_hosted::X86_64HostedBackend;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsmMode {
    Executable,
    Object,
}
