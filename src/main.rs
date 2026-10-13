mod app;
mod cli;
mod resources;

fn main() {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("rustage: {error}\nTry `rustage --help` for usage.");
            std::process::exit(2);
        }
    };

    app::run(command);
}
