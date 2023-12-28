//! Utilities for decoding and encoding data types from and to network order.
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub use codec_derive::Decode;
pub use codec_derive::Encode;

pub mod decode;
pub mod encode;

pub use decode::Decode;
pub use encode::Encode;

/// A byte array prepended with it's size which is of type `Size`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Opaque<Size> {
    inner: Vec<u8>,
    phantom: PhantomData<Size>,
}

impl<Size> Opaque<Size> {
    /// Create an empty instance of this byte array type.
    pub fn new() -> Self {
        Self {
            inner: Vec::new(),
            phantom: PhantomData,
        }
    }

    /// Extract the byte array as a `Vec<u8>`, ignoring the `Size` type.
    pub fn into_inner(self) -> Vec<u8> {
        self.inner
    }
}

impl<Size> Default for Opaque<Size> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Size> From<Vec<u8>> for Opaque<Size> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            inner: vec,
            phantom: PhantomData,
        }
    }
}

impl<Size> Deref for Opaque<Size> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner
    }
}

impl<Size> DerefMut for Opaque<Size> {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.inner
    }
}

/// A type that has his size in bytes prepended using `Size` as the type for the size.
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeWrapper<Size, T> {
    inner: T,
    phantom: PhantomData<Size>,
}

impl<Size, T> SizeWrapper<Size, T> {
    /// Wrap the given object so that when decoding or encoding it's size will be before it.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            phantom: PhantomData,
        }
    }

    /// Extract the inner type.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<Size, T> Deref for SizeWrapper<Size, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<Size, T> DerefMut for SizeWrapper<Size, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<Size, T: Default> Default for SizeWrapper<Size, T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// An array of elements of type `T` that are prepended with their total size in bytes, using
/// `Size` as the type for the size.
pub type Vector<Size, T> = SizeWrapper<Size, Vec<T>>;

/// A type that when decoded will eat the whole remaining data from `ReadBuffer`.
///
/// Trying to encode this will panic.
#[derive(Clone, Debug)]
pub struct Ignore;
