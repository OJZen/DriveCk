use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(driveck_cli::run_env() as u8)
}
