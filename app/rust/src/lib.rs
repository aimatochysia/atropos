#![deny(unsafe_code)]

#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod android;
pub mod api;
pub(crate) mod card;
pub(crate) mod crypto;
pub mod error;
mod util;
pub(crate) mod vault;
pub(crate) mod view;
#[rustfmt::skip]
#[allow(unsafe_code)]
mod frb_generated;
