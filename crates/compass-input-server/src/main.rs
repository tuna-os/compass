//! `vicinae-input-server`: see the library documentation.

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(compass_input_server::server::run())
}
