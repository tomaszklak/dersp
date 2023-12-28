use std::convert::identity;

use codec::decode::DecodeError;
use codec::{Decode, Vector};

#[test]
fn simple_fields() -> Result<(), DecodeError> {
    #[derive(Debug, PartialEq, Eq, Decode)]
    struct UnitStruct;
    assert_eq!(UnitStruct::decode(&mut (&[] as &[u8]))?, UnitStruct);

    let buffer: &[u8] = &[1, 2, 3, 4, 5, 6, 7];

    #[derive(Debug, PartialEq, Eq, Decode)]
    struct NamedFieldsStruct {
        one: u8,
        two: u16,
        three: u32,
    }
    assert_eq!(
        NamedFieldsStruct::decode(&mut identity(buffer))?,
        NamedFieldsStruct {
            one: 0x01,
            two: 0x0203,
            three: 0x0405_0607,
        }
    );

    #[derive(Debug, PartialEq, Eq, Decode)]
    struct UnnamedFieldsStruct(u8, u16, u32);
    assert_eq!(
        UnnamedFieldsStruct::decode(&mut identity(buffer))?,
        UnnamedFieldsStruct(0x01, 0x0203, 0x0405_0607)
    );

    Ok(())
}

#[test]
fn generic_fields() -> Result<(), DecodeError> {
    #[derive(Debug, PartialEq, Eq, Decode)]
    struct NamedWrapper<T> {
        value: T,
    }

    let buffer: &[u8] = &[0xaa, 0xbb];
    assert_eq!(
        NamedWrapper::decode(&mut identity(buffer))?,
        NamedWrapper { value: 0xaabbu16 }
    );

    #[derive(Debug, PartialEq, Eq, Decode)]
    struct UnnamedWrapper<T>(T);

    let buffer: &[u8] = &[0xcc, 0xdd];
    assert_eq!(
        UnnamedWrapper::decode(&mut identity(buffer))?,
        UnnamedWrapper(0xccddu16)
    );

    #[derive(Debug, PartialEq, Eq, Decode)]
    struct Pair<L, R> {
        left: L,
        right: R,
    }

    let buffer: &[u8] = &[0xee, 0xff];
    assert_eq!(
        Pair::decode(&mut identity(buffer))?,
        Pair {
            left: 0xeeu8,
            right: 0xffu8
        }
    );

    Ok(())
}

#[test]
fn vectors() -> Result<(), DecodeError> {
    let buffer: &[u8] = &[7, 1, 2, 3, 4, 5, 6, 7];
    assert_eq!(
        Vector::<u8, u8>::decode(&mut identity(buffer))?,
        Vector::new(vec![1, 2, 3, 4, 5, 6, 7])
    );

    let buffer: &[u8] = &[
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
    ];
    assert_eq!(
        Vector::decode(&mut identity(buffer))?,
        Vector::<u16, _>::new(vec![Vector::<u8, u16>::new(vec![1, 2]); 3])
    );
    Ok(())
}

#[test]
fn enums_simple() -> Result<(), DecodeError> {
    #[derive(Debug, PartialEq, Eq, Decode)]
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

    let mut buffer: &[u8] = &[4, 2, 1, 3];
    assert_eq!(Simple::decode(&mut buffer)?, Simple::Unknown);
    assert_eq!(Simple::decode(&mut buffer)?, Simple::Two);
    assert_eq!(Simple::decode(&mut buffer)?, Simple::One);
    assert_eq!(Simple::decode(&mut buffer)?, Simple::Three);

    #[derive(Debug, PartialEq, Eq, Decode)]
    enum UnknownUnnamed {
        #[tag(5)]
        Known,
        #[unknown]
        Unknown(#[unknown] u8),
    }

    let mut buffer: &[u8] = &[5, 6];
    assert_eq!(UnknownUnnamed::decode(&mut buffer)?, UnknownUnnamed::Known);
    assert_eq!(
        UnknownUnnamed::decode(&mut buffer)?,
        UnknownUnnamed::Unknown(6)
    );

    #[derive(Debug, PartialEq, Eq, Decode)]
    enum UnknownNamed {
        #[tag(5)]
        Known { field: u8 },
        #[unknown]
        Unknown {
            #[unknown]
            first: u16,
            second: u8,
        },
    }
    let mut buffer: &[u8] = &[0, 5, 1, 7, 8, 9];
    assert_eq!(
        UnknownNamed::decode(&mut buffer)?,
        UnknownNamed::Known { field: 1 }
    );
    assert_eq!(
        UnknownNamed::decode(&mut buffer)?,
        UnknownNamed::Unknown {
            first: 0x0708,
            second: 9
        }
    );
    Ok(())
}