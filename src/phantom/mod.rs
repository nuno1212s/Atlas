use std::marker::PhantomData;

/// A phantom type that does not impose any restrictions on T
/// This is useful for cases where we want to use PhantomData
/// but do not want to impose any variance or drop check restrictions
/// on T. By using `fn() -> T`, we ensure that T is treated as
/// covariant and does not affect the drop check analysis.
pub type FPhantom<T> = PhantomData<fn() -> T>;
