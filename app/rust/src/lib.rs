#![deny(unsafe_code)]

pub mod api;
pub(crate) mod card;
pub(crate) mod crypto;
pub mod error;
mod util;
pub(crate) mod vault;
#[rustfmt::skip]
#[allow(unsafe_code)]
mod frb_generated;
