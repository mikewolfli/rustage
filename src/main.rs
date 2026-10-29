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

    // The interface language is read and installed the same way the log level is: an unknown tag is a
    // usage error, and a language this build does not bundle is refused here rather than silently
    // answered in English (principle #67).
    let language = match cli::ui_language_of(arguments.iter()) {
        Ok(language) => language,
        Err(error) => {
            eprintln!("rustage: {error}\nTry `rustage --help` for usage.");
            std::process::exit(cli::ExitCode::Usage.code());
        }
    };
    let selection = app::locale::catalog::select(language.map(|language| language.tag()));
    if let app::locale::catalog::Selection::Refused { reason, .. } = &selection {
        eprintln!("rustage: {reason}");
        std::process::exit(cli::ExitCode::Reported.code());
    }
    app::locale::catalog::install(selection);

    let command = match cli::parse(arguments) {
        Ok(command) => command,
        Err(error) => {
            // The process's own refusal prefix is identified vocabulary (`shell.error.prefix`), so a
            // Chinese-reading person sees it in the language the rest of the process speaks.
            let message = format!("{error}\nTry `rustage --help` for usage.");
            eprintln!(
                "{}",
                app::locale::catalog::Catalog::current()
                    .fill("shell.error.prefix", &[("message", message.as_str())])
            );
            std::process::exit(cli::ExitCode::Usage.code());
        }
    };

    app::run(command);
}
