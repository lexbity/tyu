use hosted::{diag, process};
use crate::config::Config;

pub fn run(config: &Config) -> i32 {
    let status = process::run(config.fasm_path, &[config.input, config.out_path])
        .map_err(|_| diag::error_simple(2002, b"failed to run fasm"));
    
    let status = match status {
        Ok(s) => s,
        Err(_) => return 2,
    };
    if status.code != 0 {
        let _ = diag::error_simple(2003, b"fasm failed");
        return 2;
    }
    0
}
