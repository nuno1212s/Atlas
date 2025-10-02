use std::collections::btree_set::{IntoIter, Iter};
use std::collections::BTreeSet;
use std::iter;
use std::iter::Once;

#[derive(Clone, Debug)]
pub enum MaybeOrderedVec<T> {
    None,
    One(T),
    Mult(BTreeSet<T>),
}

impl<T> MaybeOrderedVec<T> {
    #[must_use]
    pub fn builder() -> MaybeOrderedVecBuilder<T> {
        MaybeOrderedVecBuilder {
            current_value: Self::None,
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self::None
    }

    #[must_use]
    pub fn from_one(member: T) -> Self {
        Self::One(member)
    }

    #[must_use]
    pub fn from_many(objects: Vec<T>) -> Self
    where
        T: Ord,
    {
        let mut result = BTreeSet::new();

        for obj in objects {
            result.insert(obj);
        }

        Self::Mult(result)
    }

    #[must_use]
    pub fn from_set(set: BTreeSet<T>) -> Self {
        Self::Mult(set)
    }

    pub fn len(&self) -> usize {
        match self {
            MaybeOrderedVec::None => 0,
            MaybeOrderedVec::One(_) => 1,
            MaybeOrderedVec::Mult(tree_set) => tree_set.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            MaybeOrderedVec::None => true,
            MaybeOrderedVec::One(_) => false,
            MaybeOrderedVec::Mult(set) => set.is_empty(),
        }
    }

    pub fn iter(&'_ self) -> ItRefOrderedMaybeVec<'_, T> {
        match self {
            MaybeOrderedVec::One(one) => ItRefOrderedMaybeVec::One(iter::once(one)),
            MaybeOrderedVec::Mult(vec) => ItRefOrderedMaybeVec::Mult(vec.iter()),
            MaybeOrderedVec::None => ItRefOrderedMaybeVec::None,
        }
    }
}

impl<'a, T> IntoIterator for &'a MaybeOrderedVec<T> {
    type Item = &'a T;
    type IntoIter = ItRefOrderedMaybeVec<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T> IntoIterator for MaybeOrderedVec<T> {
    type Item = T;
    type IntoIter = ItOrderedMaybeVec<T>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            MaybeOrderedVec::One(one) => ItOrderedMaybeVec::One(iter::once(one)),
            MaybeOrderedVec::Mult(vec) => ItOrderedMaybeVec::Mult(vec.into_iter()),
            MaybeOrderedVec::None => ItOrderedMaybeVec::None,
        }
    }
}

pub enum ItOrderedMaybeVec<T> {
    None,
    One(Once<T>),
    Mult(IntoIter<T>),
}

impl<T> Iterator for ItOrderedMaybeVec<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ItOrderedMaybeVec::None => None,
            ItOrderedMaybeVec::One(iter) => iter.next(),
            ItOrderedMaybeVec::Mult(iter) => iter.next(),
        }
    }
}

pub enum ItRefOrderedMaybeVec<'a, T> {
    None,
    One(Once<&'a T>),
    Mult(Iter<'a, T>),
}

impl<'a, T> Iterator for ItRefOrderedMaybeVec<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ItRefOrderedMaybeVec::None => None,
            ItRefOrderedMaybeVec::One(iter) => iter.next(),
            ItRefOrderedMaybeVec::Mult(iter) => iter.next(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MaybeOrderedVecBuilder<T> {
    current_value: MaybeOrderedVec<T>,
}

impl<T> MaybeOrderedVecBuilder<T> {
    #[must_use]
    pub fn empty() -> Self {
        MaybeOrderedVecBuilder {
            current_value: MaybeOrderedVec::None,
        }
    }

    pub fn from_existing(vec: MaybeOrderedVec<T>) -> Self {
        Self { current_value: vec }
    }

    pub fn push(&mut self, value: T)
    where
        T: Ord,
    {
        let current = std::mem::replace(&mut self.current_value, MaybeOrderedVec::None);

        self.current_value = match current {
            MaybeOrderedVec::None => MaybeOrderedVec::One(value),
            MaybeOrderedVec::One(curr_value) => {
                let mut btree_set = BTreeSet::new();

                btree_set.insert(curr_value);
                btree_set.insert(value);

                MaybeOrderedVec::Mult(btree_set)
            }
            MaybeOrderedVec::Mult(mut vec) => {
                vec.insert(value);

                MaybeOrderedVec::Mult(vec)
            }
        };
    }

    pub fn build(self) -> MaybeOrderedVec<T> {
        self.current_value
    }
}
