//! Auth route handler re-exports.
//! Re-exports from the auth module to make handler routing simpler.

pub use crate::auth::{login, register, me, change_password, forgot_password, reset_password};
