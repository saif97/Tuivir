//! Compatibility facade for infrastructure configuration loading.

pub use crate::infrastructure::config::{
    ConfigError, Env, FileSystemReader, LoadError, ReadFile, load,
};
