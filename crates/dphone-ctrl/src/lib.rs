//! What the updater (`dphone-update`) and the early-boot stage
//! (`dphone-early`, the first program the kernel runs, from the initramfs
//! in the kernel partition) must do in exactly the same way: check and
//! rewrite the bank-selection record in `ctrl`, and read and write the
//! trial-boot record.  No standard library and no allocation, so that the
//! early stage stays a few kilobytes.

#![no_std]

pub mod boot;
pub mod flash;
pub mod record;
pub mod trial;

pub use flash::{Flash, Geometry, Replication, Scan, WriteError, scan, write};
