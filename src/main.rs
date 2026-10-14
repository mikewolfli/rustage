mod app;
mod cli;
mod contracts;
mod host;
mod probe;
mod project;
mod resources;

fn main() {
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    // The logger is installed once, before anything can fail, so a startup that goes wrong has
    // somewhere to say so. A level the caller named but that does not parse is a usage error.
    let level = match cli::log_level_of(arguments.iter()) {
        Ok(level) => level.unwrap_or(host::log::Level::Info),
        Err(error) => {
            eprintln!("rustage: {error}\nTry `rustage --help` for usage.");
            std::process::exit(cli::ExitCode::Usage.code());
        }
    };
    host::log::install(host::log::Logger::to_stderr(level));

    let command = match cli::parse(arguments) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("rustage: {error}\nTry `rustage --help` for usage.");
            std::process::exit(cli::ExitCode::Usage.code());
        }
    };

    app::run(command);
}
