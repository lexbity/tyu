//! `.lmod` container packer — binary entry point.
//!
//! Usage:
//!   lmod-pack <input.o> <output.lmod>

use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] == "--help" || args[1] == "-h" {
        eprintln!("Usage: lmod-pack <input.o> <output.lmod>");
        process::exit(1);
    }

    let input = match fs::read(&args[1]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: read {}: {}", args[1], e);
            process::exit(2);
        }
    };

    let output = match lmod_pack::pack(&input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(2);
        }
    };

    if let Err(e) = fs::write(&args[2], &output) {
        eprintln!("error: write {}: {}", args[2], e);
        process::exit(2);
    }

    eprintln!(
        "packed {} -> {} ({} bytes)",
        args[1], args[2], output.len(),
    );
}
