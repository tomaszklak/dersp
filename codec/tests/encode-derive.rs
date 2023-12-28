use std::panic;

use codec::encode::BufferOverflow;
use codec::{Encode, Vector};

#[test]
fn simple_fields() {
    #[derive(Encode)]
    struct UnitStruct;
    let mut buffer = Vec::new();
    assert_eq!(UnitStruct.encode(&mut buffer), Ok(0));
    assert_eq!(buffer, Vec::new());

    #[derive(Encode)]
    struct NamedFieldsStruct {
        three: u32,
        one: u8,
        two: u16,
    }
    let mut buffer = Vec::new();
    let value = NamedFieldsStruct {
        one: 0x01,
        two: 0x0203,
        three: 0x0405_0607,
    };
    assert_eq!(value.encode(&mut buffer), Ok(7));
    assert_eq!(buffer, vec![4, 5, 6, 7, 1, 2, 3]);

    #[derive(Encode)]
    struct UnnamedFieldsStruct(u32, u16, u8);
    let mut buffer = Vec::new();
    let value = UnnamedFieldsStruct(0x0405_0607, 0x0203, 0x01);
    assert_eq!(value.encode(&mut buffer), Ok(7));
    assert_eq!(buffer, vec![4, 5, 6, 7, 2, 3, 1]);
}

#[test]
fn generic_fields() {
    #[derive(Encode)]
    struct NamedWrapper<T> {
        value: T,
    }
    let value = NamedWrapper { value: 0xaabbu16 };
    let mut buffer = Vec::new();
    assert_eq!(value.encode(&mut buffer), Ok(2));
    assert_eq!(buffer, vec![0xaa, 0xbb]);

    #[derive(Encode)]
    struct UnnamedWrapper<T>(T);
    let mut buffer = Vec::new();
    assert_eq!(UnnamedWrapper(0xccddu16).encode(&mut buffer), Ok(2));
    assert_eq!(buffer, vec![0xcc, 0xdd]);

    #[derive(Encode)]
    struct Pair<L, R> {
        right: R,
        left: L,
    }
    let pair = Pair {
        left: 0xeeu8,
        right: 0xabbacdefu32,
    };
    let mut buffer = Vec::new();
    assert_eq!(pair.encode(&mut buffer), Ok(5));
    assert_eq!(buffer, vec![0xab, 0xba, 0xcd, 0xef, 0xee]);
}

#[test]
fn vectors() {
    let mut buffer = Vec::new();
    assert_eq!(
        Vector::<u8, u8>::new(vec![1, 2, 3, 4, 5, 6, 7]).encode(&mut buffer),
        Ok(8)
    );
    assert_eq!(buffer, vec![7, 1, 2, 3, 4, 5, 6, 7]);

    let matrix = Vector::<u16, _>::new(vec![Vector::<u8, u16>::new(vec![1, 2]); 3]);

    let mut buffer = Vec::new();
    assert_eq!(matrix.encode(&mut buffer), Ok(2 + (1 + 2 * 2) * 3));
    assert_eq!(
        buffer,
        vec![
            0,
            (1 + 2 * 2) * 3,
            2 * 2,
            0,
            1,
            0,
            2,
            2 * 2,
            0,
            1,
            0,
            2,
            2 * 2,
            0,
            1,
            0,
            2,
        ]
    );
}

#[test]
fn encode_in_slice() {
    let mut slice = [0; 7];

    let mut view: &mut [u8] = &mut slice;
    assert_eq!(0x01u8.encode(&mut view), Ok(1));
    assert_eq!(0x0203u16.encode(&mut view), Ok(2));
    assert_eq!(0x0405_0607u32.encode(&mut view), Ok(4));
    assert_eq!(0x08u8.encode(&mut view), Err(BufferOverflow));
    assert_eq!((&[][..]).encode(&mut view), Ok(0));
    assert_eq!(slice, [1, 2, 3, 4, 5, 6, 7]);

    let mut slice = [0; 2];
    let mut view: &mut [u8] = &mut slice;
    #[derive(Encode)]
    struct SliceRef<'a>(&'a [u8]);
    assert_eq!(SliceRef(&[9, 10]).encode(&mut view), Ok(2));
    assert_eq!(slice, [9, 10]);
}

#[test]
fn enums_simple() {
    #[derive(Encode)]
    enum Simple {
        #[tag(1u8)]
        One,
        #[tag(2)]
        Two,
        #[tag(3)]
        Three,
        #[unknown]
        Unknown,
    }
    let mut buffer = Vec::new();
    assert_eq!(Simple::One.encode(&mut buffer), Ok(1));
    assert_eq!(Simple::Two.encode(&mut buffer), Ok(1));
    assert_eq!(Simple::Three.encode(&mut buffer), Ok(1));
    assert_eq!(buffer, vec![1, 2, 3]);
    assert!(panic::catch_unwind(move || Simple::Unknown.encode(&mut buffer)).is_err());

    #[derive(Encode)]
    enum UnknownUnnamed {
        #[tag(5u16)]
        Known,
        #[unknown]
        Unknown(#[unknown] u16),
    }
    let mut buffer = Vec::new();
    assert_eq!(UnknownUnnamed::Known.encode(&mut buffer), Ok(2));
    assert_eq!(UnknownUnnamed::Unknown(0xffff).encode(&mut buffer), Ok(2));
    assert_eq!(buffer, vec![0, 5, 0xff, 0xff]);

    #[derive(Encode)]
    enum UnknownNamed {
        #[tag(10u32)]
        Known,
        #[unknown]
        Unknown {
            #[unknown]
            tag: u32,
            extra: u16,
        },
    }

    let mut buffer = Vec::new();
    assert_eq!(UnknownNamed::Known.encode(&mut buffer), Ok(4));
    assert_eq!(
        UnknownNamed::Unknown {
            tag: 0xaabbccdd,
            extra: 0x0102
        }
        .encode(&mut buffer),
        Ok(6)
    );
    assert_eq!(
        buffer,
        vec![0, 0, 0, 10, 0xaa, 0xbb, 0xcc, 0xdd, 0x01, 0x02]
    );
}