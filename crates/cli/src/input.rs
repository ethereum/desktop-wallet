use std::{
    io::{self, BufRead, IsTerminal, Write},
    sync::Mutex,
};

use anyhow::Context;
use zeroize::Zeroizing;

use crate::failure;

static PROMPTED: Mutex<Prompted> = Mutex::new(Prompted {
    words: Vec::new(),
    asked: false,
});

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

/// One entry of a list offered by [`Input::choose`].
pub struct Choice {
    label: String,
    answer: Vec<String>,
}

struct Prompted {
    words: Vec<String>,
    asked: bool,
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

    /// The answer to the prompt for `flag`, typed at a terminal or read from piped stdin. A
    /// terminal answer, empty included, goes into [`replay`]. Never ask this way for a secret.
    pub fn ask(self, flag: &'static str, label: &str) -> anyhow::Result<Option<String>> {
        let answer = self.line(label)?;
        if let Some(answer) = &answer {
            self.record(flag_words(flag, answer.clone()), true);
        }
        Ok(answer)
    }

    /// As [`Self::ask`], where an empty answer is `default` and is recorded as it.
    pub fn ask_or(
        self,
        flag: &'static str,
        label: &str,
        default: &str,
    ) -> anyhow::Result<Option<String>> {
        let answer = self.line(label)?.map(|answer| {
            if answer.is_empty() {
                default.to_string()
            } else {
                answer
            }
        });
        if let Some(answer) = &answer {
            self.record(flag_words(flag, answer.clone()), true);
        }
        Ok(answer)
    }

    /// Whether `question` was answered yes. Never recorded, so a replayed command asks again.
    pub fn confirm(self, question: &str) -> anyhow::Result<bool> {
        let answer = self.line(question)?.unwrap_or_default();
        Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
    }

    /// The index of one of `choices`, picked by number from a list at an interactive terminal.
    /// Anywhere else there is nobody to pick, so the caller names the flag instead.
    pub fn choose(self, heading: &str, choices: &[Choice]) -> anyhow::Result<Option<usize>> {
        self.pick(heading, choices, None)
    }

    /// As [`Self::choose`], where an empty answer picks `default`.
    pub fn choose_or(
        self,
        heading: &str,
        choices: &[Choice],
        default: usize,
    ) -> anyhow::Result<Option<usize>> {
        self.pick(heading, choices, Some(default))
    }

    /// Records `flag` for a choice made without asking that a rerun could make differently, such
    /// as the only funded payer. It alone does not make [`replay`] print anything.
    pub fn imply(self, flag: &'static str, value: Option<String>) {
        let words = match value {
            Some(value) => flag_words(flag, value),
            None => vec![flag.to_string()],
        };
        self.record(words, false);
    }

    fn line(self, label: &str) -> anyhow::Result<Option<String>> {
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

    fn pick(
        self,
        heading: &str,
        choices: &[Choice],
        default: Option<usize>,
    ) -> anyhow::Result<Option<usize>> {
        if self != Self::Terminal {
            return Ok(None);
        }
        eprintln!("{heading}");
        for (index, choice) in choices.iter().enumerate() {
            let mark = if default == Some(index) {
                " (default)"
            } else {
                ""
            };
            eprintln!("  {}) {}{mark}", index + 1, choice.label);
        }
        let Some(answer) = self.line("> ")? else {
            return Ok(None);
        };
        let index = match default {
            Some(default) if answer.is_empty() => default,
            _ => answer
                .parse::<usize>()
                .ok()
                .filter(|number| (1..=choices.len()).contains(number))
                .map(|number| number - 1)
                .ok_or_else(|| failure::usage(format!("`{answer}` is not one of the choices")))?,
        };
        let answer = &choices[index].answer;
        if !answer.is_empty() {
            self.record(answer.clone(), true);
        }
        Ok(Some(index))
    }

    fn record(self, words: Vec<String>, asked: bool) {
        if self != Self::Terminal {
            return;
        }
        if let Ok(mut prompted) = PROMPTED.lock() {
            prompted.words.extend(words);
            prompted.asked |= asked;
        }
    }
}

impl Choice {
    /// A choice that `flag value` makes without asking.
    pub fn new(label: impl Into<String>, flag: &'static str, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            answer: flag_words(flag, value.into()),
        }
    }

    /// A choice that `flag` alone makes without asking.
    pub fn flag(label: impl Into<String>, flag: &'static str) -> Self {
        Self {
            label: label.into(),
            answer: vec![flag.to_string()],
        }
    }

    /// A choice no flag of the running command makes, such as the network an unlock opens.
    pub fn unrecorded(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            answer: Vec::new(),
        }
    }
}

/// The command line with each recorded answer as a flag, or `None` when nothing was asked. It
/// leaves out `--yes`, so running it still asks before broadcasting.
pub fn replay() -> Option<String> {
    let prompted = PROMPTED.lock().ok()?;
    if !prompted.asked {
        return None;
    }
    let given = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    Some(command_line(given, &prompted.words))
}

/// Clap reads every word after `--` as a positional, so recorded flags go before it.
fn command_line(given: impl Iterator<Item = String>, recorded: &[String]) -> String {
    let given: Vec<String> = given.collect();
    let end_of_options = given
        .iter()
        .position(|word| word == "--")
        .unwrap_or(given.len());
    let (options, positionals) = given.split_at(end_of_options);
    let words: Vec<String> = std::iter::once("edw".to_string())
        .chain(
            options
                .iter()
                .filter(|word| *word != "--yes")
                .chain(recorded)
                .chain(positionals)
                .map(|word| quote(word)),
        )
        .collect();
    words.join(" ")
}

/// `flag` and `value` as command-line words. Clap reads a separate word starting with `-` as a
/// flag, never as the value before it, so such a value is attached with `=`.
pub fn flag_words(flag: &str, value: String) -> Vec<String> {
    if value.starts_with('-') {
        vec![format!("{flag}={value}")]
    } else {
        vec![flag.to_string(), value]
    }
}

/// zsh expands a word starting with `=` to a command's path.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && !word.starts_with('=')
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:,@=+%".contains(&byte));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn quote_leaves_plain_words_and_single_quotes_the_rest() {
        assert_eq!(quote("--amount"), "--amount");
        assert_eq!(quote("0x11:0x:1,0x22:0x"), "0x11:0x:1,0x22:0x");
        assert_eq!(quote("my wallet"), "'my wallet'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("=ls"), "'=ls'");
    }

    #[test]
    fn a_command_line_never_carries_yes() {
        let given = ["transfer", "--yes", "--from", "0"].map(String::from);
        assert_eq!(
            command_line(given.into_iter(), &["--amount".into(), "1".into()]),
            "edw transfer --from 0 --amount 1"
        );
    }

    #[test]
    fn a_command_line_after_an_end_of_options_separator_still_parses() {
        let given = ["profile", "rename", "--", "-renamed"].map(String::from);
        let line = command_line(given.into_iter(), &["--profile".into(), "0/0".into()]);

        let words = line.split(' ');
        assert!(crate::Cli::try_parse_from(words).is_ok(), "{line}");
    }
}
