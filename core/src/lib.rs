//! Pure, dependency-light logic for MarcDigital, kept separate from the SDL2 /
//! Azure binary so it can be unit-tested without any system libraries.

pub mod config;
pub mod store;
pub mod sync;
