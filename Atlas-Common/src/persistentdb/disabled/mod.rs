//! A no-op key-value store, used when no persistent database backend is selected.
//!
//! Every read reports "not present" and every write succeeds without storing
//! anything. This lets the middleware run with persistence entirely disabled
//! (useful for benchmarking) without any call site having to know about it.

use crate::error::*;
use crate::persistentdb::{IteratorUtil, KeyValueEntry};
use std::path::Path;

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct DisabledKV;

/// Always-empty iterator, so `iter`/`iter_range` can satisfy [`IteratorUtil`].
pub struct DisabledKVIterator;

impl Iterator for DisabledKVIterator {
    type Item = Result<KeyValueEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        None
    }
}

impl IteratorUtil for DisabledKVIterator {
    type ItemType = Box<[u8]>;
}

#[allow(dead_code)]
impl DisabledKV {
    pub fn new<T>(_db_location: T, _prefixes: Vec<&'static str>) -> Result<Self>
    where
        T: AsRef<Path>,
    {
        Ok(DisabledKV)
    }

    pub fn get<T>(&self, _prefix: &'static str, _key: T) -> Result<Option<Vec<u8>>>
    where
        T: AsRef<[u8]>,
    {
        Ok(None)
    }

    pub fn get_all<T, Y>(&self, _prefix: &'static str, keys: T) -> Result<Vec<Option<Vec<u8>>>>
    where
        T: Iterator<Item = Y>,
        Y: AsRef<[u8]>,
    {
        Ok(keys.map(|_| None).collect())
    }

    pub fn exists<T>(&self, _prefix: &'static str, _key: T) -> Result<bool>
    where
        T: AsRef<[u8]>,
    {
        Ok(false)
    }

    pub fn set<T, Y>(&self, _prefix: &'static str, _key: T, _data: Y) -> Result<()>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        Ok(())
    }

    pub fn set_all<T, Y, Z>(&self, _prefix: &'static str, _values: T) -> Result<()>
    where
        T: Iterator<Item = (Y, Z)>,
        Y: AsRef<[u8]>,
        Z: AsRef<[u8]>,
    {
        Ok(())
    }

    pub fn erase<T>(&self, _prefix: &'static str, _key: T) -> Result<Option<Vec<u8>>>
    where
        T: AsRef<[u8]>,
    {
        Ok(None)
    }

    /// Delete a set of keys
    /// Accepts an [`&[&[u8]]`], in any possible form, as long as it can be dereferenced
    /// all the way to the intended target.
    pub fn erase_keys<T, Y>(&self, _prefix: &'static str, _keys: T) -> Result<()>
    where
        T: Iterator<Item = Y>,
        Y: AsRef<[u8]>,
    {
        Ok(())
    }

    pub fn erase_range<T>(&self, _prefix: &'static str, _start: T, _end: T) -> Result<()>
    where
        T: AsRef<[u8]>,
    {
        Ok(())
    }

    pub fn compact_range<T, Y>(
        &self,
        _prefix: &'static str,
        _start: Option<T>,
        _end: Option<Y>,
    ) -> Result<()>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        Ok(())
    }

    pub fn iter(&self, _prefix: &'static str) -> Result<impl IteratorUtil + '_> {
        Ok(DisabledKVIterator)
    }

    pub fn iter_range<T, Y>(
        &self,
        _prefix: &'static str,
        _start: Option<T>,
        _end: Option<Y>,
    ) -> Result<impl IteratorUtil + '_>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        Ok(DisabledKVIterator)
    }
}
