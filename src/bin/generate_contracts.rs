//! Deterministic contract snapshot command.

#![forbid(unsafe_code)]

use std::{env, error::Error, io, path::Path};

#[path = "../contract_generation/mod.rs"]
mod contract_generation;

fn main() {
    if let Err(error) = run() {
        eprintln!("contract generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [command, output_flag, output, provenance_flag, provenance]
            if command == "generate"
                && output_flag == "--output"
                && provenance_flag == "--provenance" =>
        {
            let count = contract_generation::generate(Path::new(output), Path::new(provenance))?;
            println!("generated {count} deterministic contract artifacts");
            Ok(())
        }
        [command, input_flag, input, provenance_flag, provenance]
            if command == "verify"
                && input_flag == "--input"
                && provenance_flag == "--provenance" =>
        {
            let count = contract_generation::verify(Path::new(input), Path::new(provenance))?;
            println!("verified {count} deterministic contract artifacts");
            Ok(())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: generate_contracts generate --output <dir> --provenance <file> | verify --input <dir> --provenance <file>",
        )
        .into()),
    }
}
