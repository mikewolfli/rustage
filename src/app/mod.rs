use crate::cli::{self, Command};

pub fn run(command: Command) {
    match command {
        Command::Help => print!("{}", cli::help_text()),
        Command::Version => println!("rustage {}", env!("CARGO_PKG_VERSION")),
        Command::Status => println!(
            "rustage {}\n\
             This is the initial executable scaffold; the desktop designer is not implemented yet.",
            env!("CARGO_PKG_VERSION")
        ),
    }
}
