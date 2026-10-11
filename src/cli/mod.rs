use std::{
    ffi::{OsStr, OsString},
    fmt,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Status,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    argument: OsString,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unexpected argument `{}`",
            self.argument.to_string_lossy()
        )
    }
}

impl std::error::Error for ParseError {}

pub fn parse<I, S>(arguments: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut arguments = arguments.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };

    let command = match argument.as_ref() {
        value if value == "--help" || value == "-h" => Command::Help,
        value if value == "--version" || value == "-V" => Command::Version,
        _ => {
            return Err(ParseError {
                argument: argument.as_ref().to_os_string(),
            });
        }
    };

    if let Some(argument) = arguments.next() {
        return Err(ParseError {
            argument: argument.as_ref().to_os_string(),
        });
    }

    Ok(command)
}

pub fn help_text() -> &'static str {
    "rustage - Rust desktop UI designer\n\n\
     Usage: rustage [--help | --version]\n\n\
     With no options, prints the current project status.\n\n\
     Options:\n\
       -h, --help       Print this help message\n\
       -V, --version    Print the application version\n"
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    #[test]
    fn no_arguments_show_status() {
        assert_eq!(parse([] as [&str; 0]), Ok(Command::Status));
    }

    #[test]
    fn parses_help_and_version_options() {
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["-h"]), Ok(Command::Help));
        assert_eq!(parse(["--version"]), Ok(Command::Version));
        assert_eq!(parse(["-V"]), Ok(Command::Version));
    }

    #[test]
    fn rejects_unknown_and_extra_arguments() {
        assert!(parse(["--launch"]).is_err());
        assert!(parse(["--help", "--version"]).is_err());
    }
}
