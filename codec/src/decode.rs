//! Network order decoding of types.
use std::convert::Infallible;
use std::fmt::Debug;
use std::mem;

use crate::{Ignore, Opaque, SizeWrapper};

/// The error returned by a read buffer when it has insufficient bytes.
#[derive(Debug)]
pub struct DecodeError;

/// A read buffer where data can be decoded from.
pub trait ReadBuffer {
    /// The error returned by this library if there are insufficient bytes during decoding.
    type Error: From<DecodeError>;

    /// Returns whether or not the current read buffer has any more bytes left to be extracted.
    fn is_empty(&self) -> bool;

    /// Fill a buffer of size `size` from this read buffer.
    fn fill_buf(&mut self, size: usize) -> Result<&[u8], Self::Error>;

    /// Return all available bytes in this read buffer.
    fn fill_all(&mut self) -> &[u8];
}

impl ReadBuffer for &[u8] {
    type Error = DecodeError;

    fn is_empty(&self) -> bool {
        <[u8]>::is_empty(self)
    }

    fn fill_buf(&mut self, size: usize) -> Result<&[u8], Self::Error> {
        if self.len() < size {
            return Err(DecodeError);
        }

        let (current, left) = self.split_at(size);
        *self = left;
        Ok(current)
    }

    fn fill_all(&mut self) -> &[u8] {
        mem::replace(self, &[])
    }
}

/// An interface for types that can be decoded from network ordered bytes
///
/// There is a derive macro provided in `codec_derive` that automatically generates
/// implementations for custom structs and enums.
pub trait Decode: Sized {
    /// Decode the current type from the given `read_buffer`, reading bytes from it in network
    /// order.
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error>;
}

impl Decode for u8 {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        read_buffer.fill_buf(1).map(|buf| buf[0])
    }
}

impl Decode for u16 {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        read_buffer
            .fill_buf(2)
            .map(|buf| (u16::from(buf[0]) << 8) + u16::from(buf[1]))
    }
}

impl Decode for u32 {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        read_buffer.fill_buf(4).map(|buf| {
            (u32::from(buf[0]) << 24)
                + (u32::from(buf[1]) << 16)
                + (u32::from(buf[2]) << 8)
                + u32::from(buf[3])
        })
    }
}

impl Decode for () {
    fn decode<R: ReadBuffer>(_: &mut R) -> Result<Self, R::Error> {
        Ok(())
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        if read_buffer.is_empty() {
            Ok(None)
        } else {
            T::decode(read_buffer).map(Some)
        }
    }
}

impl<Size: Into<usize> + Decode> Decode for Opaque<Size> {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        let len = Size::decode(read_buffer)?.into();
        read_buffer
            .fill_buf(len)
            .map(<[u8]>::to_vec)
            .map(Opaque::from)
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        let mut vector = Vec::new();

        while !read_buffer.is_empty() {
            vector.push(T::decode(read_buffer)?);
        }

        Ok(vector)
    }
}

// This will fail if size of Size is bigger than size of usize
impl<Size: TryInto<usize> + Decode, T: Decode> Decode for SizeWrapper<Size, T>
where
    <Size as TryInto<usize>>::Error: Debug,
{
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        let size = Size::decode(read_buffer)?
            .try_into()
            .map_err(|_| DecodeError)?;

        let left = &mut read_buffer.fill_buf(size)?;

        let value = T::decode(left)?;

        if left.is_empty() {
            Ok(SizeWrapper::new(value))
        } else {
            Err(DecodeError.into())
        }
    }
}

impl<const SIZE: usize> Decode for [u8; SIZE] {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        let mut array = [0; SIZE];
        array.copy_from_slice(read_buffer.fill_buf(SIZE)?);
        Ok(array)
    }
}

impl Decode for Ignore {
    fn decode<R: ReadBuffer>(read_buffer: &mut R) -> Result<Self, R::Error> {
        read_buffer.fill_all();

        Ok(Ignore)
    }
}

impl Decode for Infallible {
    fn decode<R: ReadBuffer>(_: &mut R) -> Result<Self, R::Error> {
        Err(DecodeError.into())
    }
}
