//! Network order encoding of types.
use std::convert::{Infallible, TryFrom};
use std::fmt::Debug;
use std::mem;
use std::slice;

use crate::{u24, Ignore, Opaque, SizeWrapper};

/// The error returned by a slice when it is full and no more data can be encoded into it.
#[derive(Debug, PartialEq, Eq)]
pub struct BufferOverflow;

/// A write buffer where data can be encoded into.
pub trait WriteBuffer {
    /// The error returned by this write buffer if it does not have any more space left to fill
    /// from a buffer.
    type Error;

    /// Try to fill this write buffer with the bytes from `buffer`.
    fn fill_from(&mut self, buffer: &[u8]) -> Result<(), Self::Error>;

    /// Skip `len` bytes and call `callback` on the remaining `WriteBuffer`.
    /// Then return the the `len` bytes that were skipped.Error
    ///
    /// This can be used to prepend the size in bytes before some types that do not have their
    /// total size known in advance.
    fn later_fill<C>(&mut self, len: usize, callback: C) -> Result<&mut [u8], Self::Error>
    where
        C: FnMut(&mut Self) -> Result<(), Self::Error>;
}

impl WriteBuffer for &mut [u8] {
    type Error = BufferOverflow;

    fn fill_from(&mut self, buffer: &[u8]) -> Result<(), Self::Error> {
        if self.len() < buffer.len() {
            return Err(BufferOverflow);
        }

        let (current, left) = mem::replace(self, &mut []).split_at_mut(buffer.len());
        current.copy_from_slice(buffer);
        *self = left;
        Ok(())
    }

    fn later_fill<C>(&mut self, len: usize, callback: C) -> Result<&mut [u8], Self::Error>
    where
        C: FnOnce(&mut Self) -> Result<(), Self::Error>,
    {
        if self.len() < len {
            return Err(BufferOverflow);
        }

        let (current, mut left) = mem::replace(self, &mut []).split_at_mut(len);
        callback(&mut left)?;
        *self = left;
        Ok(current)
    }
}

impl WriteBuffer for Vec<u8> {
    type Error = Infallible;

    fn fill_from(&mut self, buffer: &[u8]) -> Result<(), Self::Error> {
        self.extend_from_slice(buffer);
        Ok(())
    }

    fn later_fill<C>(&mut self, len: usize, callback: C) -> Result<&mut [u8], Self::Error>
    where
        C: FnOnce(&mut Self) -> Result<(), Self::Error>,
    {
        let start = self.len();
        self.resize(self.len() + len, 0);
        callback(self)?;
        Ok(&mut self[start..start + len])
    }
}

/// An interface for types that could represent sizes in the TLS protocol.
pub trait DataSize: TryFrom<usize> + Encode {
    /// The number of bytes this type uses on the wire.
    ///
    /// This needs to be known in advance, so they can be skipped during encoding until the total
    /// size is known.
    const BYTE_SIZE: usize = std::mem::size_of::<Self>();
}

impl DataSize for u8 {}
impl DataSize for u16 {}
impl DataSize for u24 {
    const BYTE_SIZE: usize = 3;
}
impl DataSize for u32 {}

/// An interface for types that can be encoded in network order, for use in the TLS protocol.
///
/// There is a derive macro provided in `codec_derive` that automatically generates `Encode`
/// implementations for custom structs and enums.
pub trait Encode {
    /// Encode `self` into the `WriteBuffer` in network order.
    ///
    /// This can only fail if the write buffer errors out during some operation.
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error>;
}

impl Encode for u8 {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(slice::from_ref(self))?;
        Ok(1)
    }
}

impl Encode for u16 {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(&self.to_be_bytes())?;
        Ok(2)
    }
}

impl Encode for u24 {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(&self.0.to_be_bytes()[1..4])?;
        Ok(3)
    }
}

impl Encode for u32 {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(&self.to_be_bytes())?;
        Ok(4)
    }
}

impl Encode for u64 {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(&self.to_be_bytes())?;
        Ok(8)
    }
}

impl Encode for () {
    fn encode<W: WriteBuffer>(&self, _: &mut W) -> Result<usize, W::Error> {
        Ok(0)
    }
}

impl<T: Encode> Encode for Option<T> {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        self.as_ref()
            .map(|value| value.encode(write_buffer))
            .unwrap_or(Ok(0))
    }
}

impl<'a, T: Encode + ?Sized> Encode for &'a T {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        (*self).encode(write_buffer)
    }
}

impl<Size: DataSize> Encode for Opaque<Size>
where
    <Size as TryFrom<usize>>::Error: Debug,
{
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        Ok(Size::try_from(self.len()).unwrap().encode(write_buffer)?
            + self.inner.encode(write_buffer)?)
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        let mut total = 0;
        for elem in self {
            total += elem.encode(write_buffer)?;
        }
        Ok(total)
    }
}

impl<Size: DataSize, T: Encode> Encode for SizeWrapper<Size, T>
where
    <Size as TryFrom<usize>>::Error: Debug,
{
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        let mut total = 0;
        let size_buffer = &mut write_buffer.later_fill(Size::BYTE_SIZE, |write_buffer| {
            total = self.inner.encode(write_buffer)?;
            Ok(())
        })?;
        Size::try_from(total).unwrap().encode(size_buffer).unwrap();
        Ok(total + Size::BYTE_SIZE)
    }
}

impl Encode for [u8; 46] {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(self)?;
        Ok(46)
    }
}

impl Encode for [u8; 32] {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(self)?;
        Ok(32)
    }
}

impl Encode for [u8; 24] {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(self)?;
        Ok(24)
    }
}

impl Encode for [u8; 8] {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(self)?;
        Ok(8)
    }
}

impl Encode for [u8; 4] {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(self)?;
        Ok(4)
    }
}

impl Encode for [u8] {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        write_buffer.fill_from(self)?;
        Ok(self.len())
    }
}

impl Encode for Ignore {
    fn encode<W: WriteBuffer>(&self, _: &mut W) -> Result<usize, W::Error> {
        panic!("Can not encode `Ignore`");
    }
}

impl Encode for Infallible {
    fn encode<W: WriteBuffer>(&self, _: &mut W) -> Result<usize, W::Error> {
        panic!("Can not encode `Infallible`");
    }
}

impl<A: Encode, B: Encode> Encode for (A, B) {
    fn encode<W: WriteBuffer>(&self, write_buffer: &mut W) -> Result<usize, W::Error> {
        let (a, b) = self;
        Ok(a.encode(write_buffer)? + b.encode(write_buffer)?)
    }
}
