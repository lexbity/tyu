use std::path::PathBuf;

use lang_symtab_gen::{extract_runtime_symbols, render_asm, render_names, AsmFlavor};

fn main() {
    if let Err(e) = run() {
        eprintln!("lang-symtab-gen: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("missing input object"))?;
    let output = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("missing output assembly"))?;

    let mut flavor = None;
    let mut names_out = None;
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--format" => {
                let value = args.next().ok_or_else(|| usage("missing --format value"))?;
                flavor = Some(parse_flavor(&value.to_string_lossy())?);
            }
            "--names-out" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage("missing --names-out value"))?;
                names_out = Some(PathBuf::from(value));
            }
            other => return Err(usage(&format!("unknown argument {other}"))),
        }
    }

    let flavor = flavor.ok_or_else(|| usage("missing --format"))?;
    let bytes = std::fs::read(&input).map_err(|e| format!("reading '{}': {e}", input.display()))?;
    let symbols = extract_runtime_symbols(&bytes).map_err(|e| e.to_string())?;
    std::fs::write(&output, render_asm(&symbols, flavor))
        .map_err(|e| format!("writing '{}': {e}", output.display()))?;
    if let Some(path) = names_out {
        std::fs::write(&path, render_names(&symbols))
            .map_err(|e| format!("writing '{}': {e}", path.display()))?;
    }

    Ok(())
}

fn parse_flavor(value: &str) -> Result<AsmFlavor, String> {
    match value {
        "fasm-x86_64" => Ok(AsmFlavor::FasmX86_64),
        "gas32" => Ok(AsmFlavor::Gas32),
        _ => Err(usage("format must be fasm-x86_64 or gas32")),
    }
}

fn usage(message: &str) -> String {
    format!(
        "{message}\nusage: lang-symtab-gen <runtime.o> <symtab.asm> --format <fasm-x86_64|gas32> [--names-out <path>]"
    )
}
