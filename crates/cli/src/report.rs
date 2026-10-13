use std::{
    fmt::Display,
    io::{self, Write},
};

use serde::Serialize;

/// A command's result, renderable either way.
///
/// The [`Display`] form is for a person to read; nothing may depend on its format.
pub trait Report: Serialize + Display {
    /// Names the JSON document, such as `edw/unlock`.
    const KIND: &'static str;
    /// Bumped whenever this kind's JSON shape changes.
    const VERSION: u32;

    /// Renders the report in `mode`, without a trailing newline.
    fn format(&self, mode: Mode) -> anyhow::Result<String> {
        Ok(match mode {
            Mode::Human => self.to_string(),
            Mode::Json => serde_json::to_string(&Envelope {
                kind: Self::KIND,
                version: Self::VERSION,
                report: self,
            })?,
        })
    }

    /// Writes the report to stdout.
    fn emit(&self, mode: Mode) -> anyhow::Result<()> {
        writeln!(io::stdout().lock(), "{}", self.format(mode)?)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub enum Mode {
    Human,
    Json,
}

#[derive(Serialize)]
struct Envelope<'a, R: ?Sized> {
    kind: &'static str,
    version: u32,
    #[serde(flatten)]
    report: &'a R,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Probe {
        name: &'static str,
    }

    impl Report for Probe {
        const KIND: &'static str = "edw/probe";
        const VERSION: u32 = 2;
    }

    impl Display for Probe {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.name)
        }
    }

    #[test]
    fn json_leads_with_kind_and_version() {
        assert_eq!(
            Probe { name: "a" }.format(Mode::Json).unwrap(),
            r#"{"kind":"edw/probe","version":2,"name":"a"}"#
        );
    }
}
