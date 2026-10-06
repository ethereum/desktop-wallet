pub use bootstrap::{
    ProfileError, bootstrap_profile, next_profile_index, profile_scope, rename_profile,
    resolve_profile, set_profile_name,
};
pub use db::{ProfileDatabaseError, ProfileDb, ProfileRecord};

pub mod bootstrap;
pub mod db;
