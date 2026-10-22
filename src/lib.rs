//! A correspondence Go server with no clocks and no result.
//!
//! The rules live in [`go`], persistence in [`store`], identity in [`auth`] and
//! HTTP in [`web`].

pub mod assets;
pub mod auth;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod go;
pub mod i18n;
pub mod store;
pub mod util;
pub mod web;
