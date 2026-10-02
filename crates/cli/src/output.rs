//! The two output modes `spec/02-cli.md` makes a contract.
//!
//! A command describes its result once, as a [`Report`], and never picks the mode, so the two
//! cannot drift apart per command.

use std::io::{self, Write};

use serde::Serialize;

/// A command's result, renderable either way.
pub trait Report: Serialize {
    /// Names the JSON document, such as `edw/unlock`.
    const KIND: &'static str;
    /// Bumped whenever this kind's JSON shape changes.
    const VERSION: u32;

    /// Writes the form a person reads. Nothing may depend on this format.
    fn render(&self, out: &mut dyn Write) -> io::Result<()>;
}

#[derive(Clone, Copy)]
pub enum Mode {
    Human,
    Json,
}

#[derive(Serialize)]
struct Envelope<'a, R> {
    kind: &'static str,
    version: u32,
    #[serde(flatten)]
    report: &'a R,
}

/// Writes `report` to stdout.
pub fn emit<R: Report>(mode: Mode, report: &R) -> Result<(), anyhow::Error> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    emit_to(&mut out, mode, report)?;
    out.flush()?;
    Ok(())
}

fn emit_to<R: Report>(out: &mut dyn Write, mode: Mode, report: &R) -> Result<(), anyhow::Error> {
    match mode {
        Mode::Human => report.render(out)?,
        Mode::Json => {
            let envelope = Envelope {
                kind: R::KIND,
                version: R::VERSION,
                report,
            };
            serde_json::to_writer(&mut *out, &envelope)?;
            out.write_all(b"\n")?;
        }
    }
    Ok(())
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
        const KIND: &'static str = "edw/sample";
        const VERSION: u32 = 3;

        fn render(&self, out: &mut dyn Write) -> io::Result<()> {
            writeln!(out, "{} x{}", self.name, self.count)
        }
    }

    #[test]
    fn json_is_one_line_led_by_kind_and_version() {
        let report = Sample {
            name: "default".into(),
            count: 2,
        };

        let mut buffer = Vec::new();
        emit_to(&mut buffer, Mode::Json, &report).unwrap();

        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "{\"kind\":\"edw/sample\",\"version\":3,\"name\":\"default\",\"count\":2}\n"
        );
    }
}
