//! The two output modes `spec/02-cli.md` makes a contract.
//!
//! A command describes its result once, as a [`Report`], and never picks the mode, so the two
//! cannot drift apart per command.

use std::io::{self, Write};

use serde::Serialize;

/// A command's result, renderable either way.
pub(crate) trait Report: Serialize {
    /// Writes the form a person reads. Nothing may depend on this format.
    fn render(&self, out: &mut dyn Write) -> io::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Human,
    Json,
}

/// Writes `report` to stdout, flushed.
///
/// # Errors
/// Propagates a write failure, and a serialization failure in [`Mode::Json`].
pub(crate) fn emit<R: Report>(mode: Mode, report: &R) -> Result<(), anyhow::Error> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    emit_to(&mut out, mode, report)?;
    out.flush()?;
    Ok(())
}

/// [`Mode::Json`] writes one line, newline-terminated.
fn emit_to<R: Report>(out: &mut dyn Write, mode: Mode, report: &R) -> Result<(), anyhow::Error> {
    match mode {
        Mode::Human => report.render(out)?,
        Mode::Json => {
            serde_json::to_writer(&mut *out, report)?;
            out.write_all(b"\n")?;
        }
    }
    Ok(())
}

pub(crate) fn refuse_prompt(what: &str, how: &str) -> anyhow::Error {
    anyhow::anyhow!("--non-interactive cannot prompt for {what}; pass {how}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Sample {
        name: String,
        count: u8,
    }

    impl Report for Sample {
        fn render(&self, out: &mut dyn Write) -> io::Result<()> {
            writeln!(out, "{} x{}", self.name, self.count)
        }
    }

    #[test]
    fn json_is_one_newline_terminated_line() {
        let report = Sample {
            name: "default".into(),
            count: 2,
        };

        let mut buffer = Vec::new();
        emit_to(&mut buffer, Mode::Json, &report).unwrap();
        let emitted = String::from_utf8(buffer).unwrap();

        assert_eq!(emitted.lines().count(), 1, "{emitted}");
        assert!(emitted.ends_with('\n'), "{emitted}");
    }
}
