use clap::Parser;
mod process_heap;

fn main() -> std::process::ExitCode {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--internal-svg-decoder")
    {
        let bytes = std::env::args_os()
            .nth(2)
            .and_then(|value| value.to_str().and_then(|value| value.parse::<usize>().ok()))
            .unwrap_or(256 * 1024 * 1024);
        process_heap::limit(bytes);
        return match ers::images::run_svg_helper() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        };
    }
    match ers::cli::run(
        ers::cli::Arguments::parse(),
        ers::cli::RuntimeEnvironment::capture(),
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
