use std::iter;
use std::iter::Once;

pub mod ordered;

/// Utility type for when we want to have a type
/// which can either be a single item, or a vec of items
/// we used this to avoid heap allocations with the overuse of vecs
/// when we only want a single item.
///
/// With this enum, we can represent both options while still maintaining the
/// speed and efficiency of stack allocation when we have a single value
#[derive(Clone, Debug)]
pub enum MaybeVec<T> {
    None,
    One(T),
    Mult(Vec<T>),
}

impl<T> MaybeVec<T> {
    #[must_use]
    pub fn from_one(member: T) -> Self {
        Self::One(member)
    }

    #[must_use]
    pub fn from_option(option: Option<T>) -> Self {
        match option {
            Some(value) => Self::One(value),
            None => Self::None,
        }
    }

    #[must_use]
    pub fn from_many(objects: Vec<T>) -> Self {
        Self::Mult(objects)
    }

    #[must_use]
    pub fn builder() -> MaybeVecBuilder<T> {
        MaybeVecBuilder::empty()
    }

    pub fn len(&self) -> usize {
        match self {
            MaybeVec::None => 0,
            MaybeVec::One(_) => 1,
            MaybeVec::Mult(vec) => vec.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            MaybeVec::One(_) => false,
            MaybeVec::Mult(vec) => vec.is_empty(),
            MaybeVec::None => true,
        }
    }

    pub fn iter(&self) -> ItRefMaybeVec<'_, T> {
        match self {
            MaybeVec::None => ItRefMaybeVec::None,
            MaybeVec::One(value) => ItRefMaybeVec::One(iter::once(value)),
            MaybeVec::Mult(vec) => ItRefMaybeVec::Mult(vec.iter()),
        }
    }

    pub fn iter_mut(&mut self) -> ItMutMaybeVec<'_, T> {
        match self {
            MaybeVec::None => ItMutMaybeVec::None,
            MaybeVec::One(value) => ItMutMaybeVec::One(iter::once(value)),
            MaybeVec::Mult(vec) => ItMutMaybeVec::Mult(vec.iter_mut()),
        }
    }

    pub fn into_vec(self) -> Vec<T> {
        match self {
            MaybeVec::None => Vec::new(),
            MaybeVec::One(val) => {
                vec![val]
            }
            MaybeVec::Mult(vec) => vec,
        }
    }

    /// Join two maybe vecs
    #[must_use]
    pub fn joining(self, other: Self) -> Self {
        match self {
            MaybeVec::None => other,
            MaybeVec::One(value) => match other {
                MaybeVec::None => MaybeVec::One(value),
                MaybeVec::One(other) => MaybeVec::Mult(vec![value, other]),
                MaybeVec::Mult(mut other_vec) => {
                    other_vec.push(value);

                    MaybeVec::Mult(other_vec)
                }
            },
            MaybeVec::Mult(mut vec) => match other {
                MaybeVec::None => MaybeVec::Mult(vec),
                MaybeVec::One(value) => {
                    vec.push(value);

                    MaybeVec::Mult(vec)
                }
                MaybeVec::Mult(mut other) => {
                    vec.append(&mut other);

                    MaybeVec::Mult(vec)
                }
            },
        }
    }
}

impl<'a, T> IntoIterator for &'a MaybeVec<T> {
    type Item = &'a T;
    type IntoIter = ItRefMaybeVec<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut MaybeVec<T> {
    type Item = &'a mut T;
    type IntoIter = ItMutMaybeVec<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> IntoIterator for MaybeVec<T> {
    type Item = T;
    type IntoIter = ItMaybeVec<T>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            MaybeVec::One(obj) => ItMaybeVec::One(iter::once(obj)),
            MaybeVec::Mult(vec) => ItMaybeVec::Mult(vec.into_iter()),
            MaybeVec::None => ItMaybeVec::None,
        }
    }
}

impl<T> FromIterator<T> for MaybeVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut maybe_vec = MaybeVec::builder();

        iter.into_iter().for_each(|item| maybe_vec.push(item));

        maybe_vec.build()
    }
}

impl<T> From<Option<T>> for MaybeVec<T> {
    fn from(option: Option<T>) -> Self {
        match option {
            Some(value) => MaybeVec::One(value),
            None => MaybeVec::None,
        }
    }
}

pub enum ItMaybeVec<T> {
    None,
    One(Once<T>),
    Mult(std::vec::IntoIter<T>),
}

impl<T> Iterator for ItMaybeVec<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ItMaybeVec::None => None,
            ItMaybeVec::One(iter) => iter.next(),
            ItMaybeVec::Mult(iter) => iter.next(),
        }
    }
}

pub enum ItMutMaybeVec<'a, T> {
    None,
    One(Once<&'a mut T>),
    Mult(std::slice::IterMut<'a, T>),
}

impl<'a, T> Iterator for ItMutMaybeVec<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ItMutMaybeVec::None => None,
            ItMutMaybeVec::One(iter) => iter.next(),
            ItMutMaybeVec::Mult(iter) => iter.next(),
        }
    }
}

pub enum ItRefMaybeVec<'a, T> {
    None,
    One(Once<&'a T>),
    Mult(std::slice::Iter<'a, T>),
}

impl<'a, T> Iterator for ItRefMaybeVec<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ItRefMaybeVec::None => None,
            ItRefMaybeVec::One(iter) => iter.next(),
            ItRefMaybeVec::Mult(iter) => iter.next(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MaybeVecBuilder<T> {
    current_value: MaybeVec<T>,
}

impl<T> MaybeVecBuilder<T> {
    pub fn empty() -> Self {
        Self {
            current_value: MaybeVec::None,
        }
    }

    pub fn from_existing(value: MaybeVec<T>) -> Self {
        Self {
            current_value: value,
        }
    }

    pub fn push(&mut self, value: T) {
        let current = std::mem::replace(&mut self.current_value, MaybeVec::None);

        self.current_value = match current {
            MaybeVec::None => MaybeVec::One(value),
            MaybeVec::One(curr_value) => MaybeVec::Mult(vec![curr_value, value]),
            MaybeVec::Mult(mut vec) => {
                vec.push(value);

                MaybeVec::Mult(vec)
            }
        };
    }

    pub fn build(self) -> MaybeVec<T> {
        self.current_value
    }
}
