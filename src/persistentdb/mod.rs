use anyhow::Context;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

use crate::error::Result;

#[cfg(feature = "persistent_db_rocksdb")]
pub mod rocksdb;

#[cfg(feature = "persistent_db_sled")]
pub mod sled;

pub mod disabled;

/// The basic implementation for the Key-Value DB used by this middleware
/// This was abstracted so we could use multiple types of databases without having
/// to perform any alterations in the code
#[derive(Clone)]
pub struct KVDB {
    _prefixes: Vec<&'static str>,
    #[cfg(feature = "persistent_db_rocksdb")]
    inner: Arc<rocksdb::RocksKVDB>,
    #[cfg(feature = "persistent_db_sled")]
    inner: Arc<sled::SledKVDB>,
    #[cfg(all(
        not(feature = "persistent_db_rocksdb"),
        not(feature = "persistent_db_sled")
    ))]
    inner: disabled::DisabledKV,
}

#[allow(dead_code)]
type KeyValueEntry = (Box<[u8]>, Box<[u8]>);

#[allow(dead_code)]
impl KVDB {
    /// Create a new instance of the database with the given path
    ///
    /// # Errors
    ///
    /// If the database cannot be created, an error will be returned
    pub fn new<T>(db_path: T, prefixes: Vec<&'static str>) -> Result<Self>
    where
        T: AsRef<Path>,
    {
        let prefixes_cpy = prefixes.clone();

        let inner = {
            #[cfg(feature = "persistent_db_rocksdb")]
            {
                Arc::new(
                    rocksdb::RocksKVDB::new(db_path, prefixes_cpy)
                        .context("Failed to create Rocks KVDB")?,
                )
            }
            #[cfg(feature = "persistent_db_sled")]
            {
                Arc::new(
                    sled::SledKVDB::new(db_path, prefixes_cpy)
                        .context("Failed to create Sled KVDB")?,
                )
            }
            #[cfg(all(
                not(feature = "persistent_db_rocksdb"),
                not(feature = "persistent_db_sled")
            ))]
            {
                disabled::DisabledKV::new(db_path, prefixes_cpy)?
            }
        };

        Ok(Self {
            _prefixes: prefixes,
            inner,
        })
    }

    /// Get the corresponding value of a given prefix + key combo in the database
    ///
    /// # Errors
    ///
    /// Errors will be returned when failing to get the prefix handle
    pub fn get<T>(&self, prefix: &'static str, key: T) -> Result<Option<impl AsRef<[u8]>>>
    where
        T: AsRef<[u8]>,
    {
        self.inner.get(prefix, key)
    }

    /// Get the corresponding value for a given set of keys
    ///
    /// # Errors
    ///
    /// Errors will be returned when failing to get the prefix handle or each
    /// of the individual keys.
    /// Keys not existing in the database will return `None`
    pub fn get_all<T, Y>(
        &self,
        prefix: &'static str,
        key: T,
    ) -> Result<Vec<Option<impl AsRef<[u8]>>>>
    where
        T: Iterator<Item = Y>,
        Y: AsRef<[u8]>,
    {
        self.inner.get_all(prefix, key)
    }

    ///Check if the given prefix + key combination exists in the database
    pub fn exists<T>(&self, prefix: &'static str, key: T) -> Result<bool>
    where
        T: AsRef<[u8]>,
    {
        self.inner.exists(prefix, key)
    }

    pub fn set<T, Y>(&self, prefix: &'static str, key: T, data: Y) -> Result<()>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        self.inner.set(prefix, key, data)
    }

    pub fn set_all<T, Y, Z>(&self, prefix: &'static str, values: T) -> Result<()>
    where
        T: Iterator<Item = (Y, Z)>,
        Y: AsRef<[u8]>,
        Z: AsRef<[u8]>,
    {
        self.inner.set_all(prefix, values)
    }

    pub fn erase<T>(&self, prefix: &'static str, key: T) -> Result<Option<impl AsRef<[u8]>>>
    where
        T: AsRef<[u8]>,
    {
        self.inner.erase(prefix, key)
    }

    /// Delete a set of keys
    /// Accepts an [`&[&[u8]]`], in any possible form, as long as it can be dereferenced
    /// all the way to the intended target.
    pub fn erase_keys<T, Y>(&self, prefix: &'static str, keys: T) -> Result<()>
    where
        T: Iterator<Item = Y>,
        Y: AsRef<[u8]>,
    {
        self.inner.erase_keys(prefix, keys)
    }

    ///Delete a range of keys from the database
    /// Accepts the start key and the end key
    /// Deletes: `[start, end[` (exclusive on the end key)
    pub fn erase_range<T>(&self, prefix: &'static str, start: T, end: T) -> Result<()>
    where
        T: AsRef<[u8]>,
    {
        self.inner.erase_range(prefix, start, end)
    }

    pub fn compact_range<T, Y>(
        &self,
        prefix: &'static str,
        start: Option<T>,
        end: Option<Y>,
    ) -> Result<()>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        self.inner.compact_range(prefix, start, end)
    }

    pub fn iter(&self, prefix: &'static str) -> Result<impl IteratorUtil + '_> {
        self.inner.iter(prefix)
    }

    /// Iterate over a range of keys
    ///
    /// # Errors
    pub fn iter_range<T, Y>(
        &self,
        prefix: &'static str,
        start: Option<T>,
        end: Option<Y>,
    ) -> Result<impl IteratorUtil + '_>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        self.inner.iter_range(prefix, start, end)
    }
}

#[derive(Error, Debug)]
pub enum PersStorageError {
    #[error("Failed to get prefix by that name {0}")]
    NoPrefix(&'static str),
}

pub const INLINE_CUTOFF: usize = 22;

pub trait IteratorUtil: Iterator<Item = Result<(Self::ItemType, Self::ItemType)>> {
    type ItemType: AsRef<[u8]>;
}
