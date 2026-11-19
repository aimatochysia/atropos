#![deny(unsafe_code)]

pub mod api;
pub(crate) mod crypto;
pub mod error;
mod util;
#[rustfmt::skip]
#[allow(unsafe_code)]
mod frb_generated;
