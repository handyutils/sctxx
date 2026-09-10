//! The `sctxx` binary.
//!
//! Everything happens in the library; `main` only maps the result to a process
//! exit code. Exit codes are a published contract (spec §3.1).

fn main() -> std::process::ExitCode {
    let code = sctxx::cli::run();
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}
