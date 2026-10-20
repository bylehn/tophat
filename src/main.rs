use std::io::{self, IsTerminal};
use std::process::ExitCode;
use tophat::check::RULE_FAMILIES;
use tophat::cli::{run, Streams, EXIT_UNABLE};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(e) => {
            eprintln!("tophat: cannot determine the current directory: {e}");
            return ExitCode::from(EXIT_UNABLE as u8);
        }
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    let interactive = stdin.is_terminal() && stdout.is_terminal();
    let code = run(
        &args,
        &cwd,
        RULE_FAMILIES,
        &mut Streams {
            input: &mut stdin.lock(),
            out: &mut stdout.lock(),
            err: &mut io::stderr().lock(),
            interactive,
        },
    );
    ExitCode::from(code as u8)
}
