#![allow(dead_code)]

use crate::error::Result;
use crate::persistentdb::IteratorUtil;
use anyhow::Context;
use sled::IVec;
use std::path::Path;

pub(crate) struct SledKVDB {
    db_handle: ::sled::Db,
}

impl SledKVDB {
    pub fn new<T>(location: T, prefixes: Vec<&'static str>) -> Result<Self>
    where
        T: AsRef<Path>,
    {
        let db_handle = sled::open(location)?;

        for prefix in prefixes {
            // Make sure all prefixes exist
            let _ = db_handle.open_tree(prefix)?;
        }

        Ok(Self { db_handle })
    }

    fn get_tree(&self, prefix: &str) -> Result<sled::Tree> {
        self.db_handle
            .open_tree(prefix)
            .context("Failed to open sled tree")
    }

    pub fn get<T>(&self, prefix: &'static str, key: T) -> Result<Option<Vec<u8>>>
    where
        T: AsRef<[u8]>,
    {
        let tree = self.get_tree(prefix)?;

        let result = tree.get(key.as_ref())?;

        Ok(result.map(|v| v.to_vec()))
    }

    pub fn get_all<T, Y>(
        &self,
        prefix: &'static str,
        keys: T,
    ) -> Result<Vec<Option<impl AsRef<[u8]>>>>
    where
        T: Iterator<Item = Y>,
        Y: AsRef<[u8]>,
    {
        let tree = self
            .get_tree(prefix)
            .context("Failed to get tree for prefix to get all items")?;

        let items = keys
            .map(|key| {
                tree.get(key.as_ref())
                    .map(|v| v.map(|v| v.to_vec()))
                    .map_err(From::from)
            })
            .collect();

        items
    }

    pub fn exists<T>(&self, prefix: &'static str, key: T) -> Result<bool>
    where
        T: AsRef<[u8]>,
    {
        let tree = self.get_tree(prefix)?;

        Ok(tree.contains_key(key.as_ref())?)
    }

    pub fn set<T, Y>(&self, prefix: &'static str, key: T, data: Y) -> Result<()>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        let tree = self.get_tree(prefix)?;

        tree.insert(key.as_ref(), data.as_ref())
            .context(format!("Failed to set key in prefix {prefix:?}"))
            .map(|_| ())
    }
    pub fn set_all<T, Y, Z>(&self, prefix: &'static str, values: T) -> Result<()>
    where
        T: Iterator<Item = (Y, Z)>,
        Y: AsRef<[u8]>,
        Z: AsRef<[u8]>,
    {
        let tree = self.get_tree(prefix)?;

        let mut batch = ::sled::Batch::default();

        for (key, value) in values {
            batch.insert(key.as_ref(), value.as_ref());
        }

        tree.apply_batch(batch)
            .context(format!("Failed to set keys in prefix {prefix:?}"))
    }

    pub fn erase<T>(&self, prefix: &'static str, key: T) -> Result<Option<impl AsRef<[u8]>>>
    where
        T: AsRef<[u8]>,
    {
        let tree = self.get_tree(prefix)?;

        tree.remove(key.as_ref())
            .context(format!("Failed to erase key in prefix {prefix:?}"))
    }

    /// Delete a set of keys
    /// Accepts an [`&[&[u8]]`], in any possible form, as long as it can be dereferenced
    /// all the way to the intended target.
    pub fn erase_keys<T, Y>(&self, prefix: &'static str, keys: T) -> Result<()>
    where
        T: Iterator<Item = Y>,
        Y: AsRef<[u8]>,
    {
        let tree = self
            .get_tree(prefix)
            .context("Failed to get tree to erase keys")?;

        let mut batch = ::sled::Batch::default();

        for key in keys {
            batch.remove(key.as_ref());
        }

        tree.apply_batch(batch)
            .context(format!("Failed to erase keys in prefix {prefix:?}"))
    }

    pub fn erase_range<T>(&self, prefix: &'static str, start: T, end: T) -> Result<()>
    where
        T: AsRef<[u8]>,
    {
        let tree = self
            .get_tree(prefix)
            .context("Failed to get tree to erase range")?;

        let mut batch = sled::Batch::default();

        tree.range(start.as_ref()..end.as_ref()).for_each(|r| {
            let (key, _) = r.unwrap();

            batch.remove(key.as_ref());
        });

        tree.apply_batch(batch)
            .context(format!("Failed to erase range in prefix {prefix:?}"))
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

    pub(super) fn iter(&self, prefix: &'static str) -> Result<impl IteratorUtil + '_> {
        let tree = self
            .get_tree(prefix)
            .context("Failed to open tree for iterating")?;

        let iter = tree.iter();

        Ok(SledKVDBIterator { iterator: iter })
    }

    pub(super) fn iter_range<T, Y>(
        &self,
        prefix: &'static str,
        start: Option<T>,
        end: Option<Y>,
    ) -> Result<impl IteratorUtil + '_>
    where
        T: AsRef<[u8]>,
        Y: AsRef<[u8]>,
    {
        let tree = self
            .get_tree(prefix)
            .context("Failed to open tree for iterating")?;

        let iter = match (start, end) {
            (Some(start), Some(end)) => tree.range(start.as_ref()..end.as_ref()),
            (Some(start), None) => tree.range(start.as_ref()..),
            (None, Some(end)) => tree.range(..end.as_ref()),
            (None, None) => tree.iter(),
        };

        Ok(SledKVDBIterator { iterator: iter })
    }
}

pub struct SledKVDBIterator {
    iterator: sled::Iter,
}

impl IteratorUtil for SledKVDBIterator {
    type ItemType = IVec;
}

impl Iterator for SledKVDBIterator {
    type Item = Result<(IVec, IVec)>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next().map(|r| r.map_err(From::from))
    }
}
