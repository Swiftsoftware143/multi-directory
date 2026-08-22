//! Auth route handler re-exports.
//! Re-exports from the auth module to make handler routing simpler.

pub use crate::auth::{change_password, forgot_password, login, me, register, reset_password};
