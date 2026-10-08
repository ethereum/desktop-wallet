use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::Context;
use zeroize::Zeroizing;

/// Where a command may read input it was not given as an argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A person at a terminal: prompt for it.
    Terminal,
    /// Redirected stdin: read it line by line, without prompts.
    Piped,
    /// `--non-interactive` at a terminal: read nothing, so a missing input is an error.
    Disabled,
}

impl Input {
    pub fn detect(non_interactive: bool) -> Self {
        if !io::stdin().is_terminal() {
            Self::Piped
        } else if non_interactive {
            Self::Disabled
        } else {
            Self::Terminal
        }
    }

    /// A line typed without echo at a terminal, or read from piped stdin.
    ///
    /// Piped stdin was never echoed to begin with, so reading it as a plain line is not a
    /// weaker path. It is what makes the commands scriptable and testable.
    pub fn secret(self, label: &str) -> anyhow::Result<Option<Zeroizing<String>>> {
        match self {
            Self::Terminal => Ok(Some(Zeroizing::new(
                rpassword::prompt_password(label).context("error reading from the terminal")?,
            ))),
            Self::Piped => {
                let mut line = Zeroizing::new(String::new());
                io::stdin()
                    .lock()
                    .read_line(&mut line)
                    .context("error reading stdin")?;
                Ok(Some(Zeroizing::new(
                    line.trim_end_matches(['\r', '\n']).to_string(),
                )))
            }
            Self::Disabled => Ok(None),
        }
    }

    /// A trimmed line, prompted with `label` at a terminal.
    pub fn line(self, label: &str) -> anyhow::Result<Option<String>> {
        match self {
            Self::Disabled => return Ok(None),
            Self::Terminal => {
                eprint!("{label}");
                io::stderr().flush()?;
            }
            Self::Piped => {}
        }
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .context("error reading stdin")?;
        Ok(Some(line.trim().to_string()))
    }

    /// The index of one of `options`, picked by number from a list at an interactive
    /// terminal. Anywhere else there is nobody to pick, so the caller names the flag instead.
    pub fn choose(self, heading: &str, options: &[String]) -> anyhow::Result<Option<usize>> {
        if self != Self::Terminal {
            return Ok(None);
        }
        eprintln!("{heading}");
        for (number, option) in (1..).zip(options) {
            eprintln!("  {number}) {option}");
        }
        let Some(choice) = self.line("> ")? else {
            return Ok(None);
        };
        choice
            .parse::<usize>()
            .ok()
            .filter(|number| (1..=options.len()).contains(number))
            .map(|number| Some(number - 1))
            .with_context(|| format!("`{choice}` is not one of the choices"))
    }
}
