use std::{error::Error, fmt};

use edw_core::database::encrypted::EncryptedDatabaseError;
use serde::Serialize;

use crate::report::Report;

/// What kind of failure ended a command, which sets its exit code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Other,
    Usage,
    Locked,
}

/// A failure the CLI raises itself, carrying its class.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct Failure {
    class: Class,
    message: String,
}

/// The `edw/error` document a failed `--non-interactive` command prints.
#[derive(Serialize)]
pub struct ErrorReport {
    class: Class,
    message: String,
}

impl Class {
    /// The class of the outermost cause in `error`'s chain that has one, else [`Class::Other`].
    fn of(error: &anyhow::Error) -> Self {
        error.chain().find_map(classify).unwrap_or(Self::Other)
    }

    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Other => 1,
            Self::Usage => 2,
            Self::Locked => 3,
        }
    }
}

impl ErrorReport {
    pub fn new(error: &anyhow::Error) -> Self {
        Self {
            class: Class::of(error),
            message: format!("{error:#}"),
        }
    }

    pub const fn class(&self) -> Class {
        self.class
    }
}

impl Report for ErrorReport {
    const KIND: &'static str = "edw/error";
    const VERSION: u32 = 1;
}

impl fmt::Display for ErrorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub fn usage(message: impl Into<String>) -> anyhow::Error {
    failure(Class::Usage, message)
}

pub fn locked(message: impl Into<String>) -> anyhow::Error {
    failure(Class::Locked, message)
}

fn failure(class: Class, message: impl Into<String>) -> anyhow::Error {
    Failure {
        class,
        message: message.into(),
    }
    .into()
}

// Core errors are `other`, except a wrong or empty password, which unlocking can fix.
fn classify(cause: &(dyn Error + 'static)) -> Option<Class> {
    if let Some(failure) = cause.downcast_ref::<Failure>() {
        return Some(failure.class);
    }
    matches!(
        cause.downcast_ref::<EncryptedDatabaseError>(),
        Some(EncryptedDatabaseError::InvalidPassword | EncryptedDatabaseError::EmptyPassword)
    )
    .then_some(Class::Locked)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use anyhow::Context;
    use edw_core::instance::InstanceError;

    use super::*;
    use crate::report::Mode;

    #[test]
    fn context_keeps_the_class_of_the_cause() {
        let error = Err::<(), _>(locked("wallet is locked"))
            .context("opening the wallet")
            .unwrap_err();
        assert_eq!(Class::of(&error), Class::Locked);
    }

    #[test]
    fn a_wrong_password_under_unlock_is_locked() {
        let error = anyhow::Error::from(InstanceError::Unlock(
            EncryptedDatabaseError::InvalidPassword,
        ));
        assert_eq!(Class::of(&error), Class::Locked);
    }

    #[test]
    fn an_unclassified_error_documents_as_other_with_its_full_message() {
        let error = anyhow::anyhow!("no profiles").context("choosing a profile");
        assert_eq!(
            ErrorReport::new(&error).format(Mode::Json).unwrap(),
            r#"{"kind":"edw/error","version":1,"class":"other","message":"choosing a profile: no profiles"}"#
        );
    }
}
