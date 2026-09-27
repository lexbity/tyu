//! `.lmod` container packer — binary entry point.
//!
//! Usage:
//!   lmod-pack [--verify-manifest <tyu.vm/1.json>] <input.o> <output.lmod>

use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut input_pos = 1usize;
    let mut manifest_path: Option<String> = None;

    while input_pos < args.len() && args[input_pos].starts_with("--") {
        match args[input_pos].as_str() {
            "--help" | "-h" => {
                eprintln!(
                    "Usage: lmod-pack [--verify-manifest <tyu.vm/1.json>] <input.o> <output.lmod>"
                );
                process::exit(0);
            }
            "--verify-manifest" => {
                if input_pos + 1 >= args.len() {
                    eprintln!("error: --verify-manifest needs a file path");
                    process::exit(2);
                }
                manifest_path = Some(args[input_pos + 1].clone());
                input_pos += 2;
            }
            other => {
                eprintln!("error: unknown flag {other}");
                process::exit(2);
            }
        }
    }

    if args.len() < input_pos + 2 {
        eprintln!("Usage: lmod-pack [--verify-manifest <tyu.vm/1.json>] <input.o> <output.lmod>");
        process::exit(1);
    }

    let input = match fs::read(&args[input_pos]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: read {}: {}", args[input_pos], e);
            process::exit(2);
        }
    };

    // Assemble the verify_manifest record (P11): read the summary, encode the
    // canonical record, fail closed on any malformed input.
    let record: Vec<u8> = match &manifest_path {
        None => Vec::new(),
        Some(path) => {
            let text = match fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: read {}: {}", path, e);
                    process::exit(2);
                }
            };
            // The single summary→record path (the same one build and deploy
            // use), so the CLI can never drift from the packed artifacts.
            match lmod_pack::verify::encode_from_json_text(&text) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: --verify-manifest: {e}");
                    process::exit(2);
                }
            }
        }
    };

    let output = match lmod_pack::pack_with_verify_manifest(&input, &record) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(2);
        }
    };

    if let Err(e) = fs::write(&args[input_pos + 1], &output) {
        eprintln!("error: write {}: {}", args[input_pos + 1], e);
        process::exit(2);
    }

    eprintln!(
        "packed {} -> {} ({} bytes, verify_manifest {})",
        args[input_pos],
        args[input_pos + 1],
        output.len(),
        if record.is_empty() {
            "absent"
        } else {
            "present"
        },
    );
}
