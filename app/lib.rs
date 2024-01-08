#[cfg(test)]
#[macro_use(quickcheck)]
extern crate quickcheck_macros;

pub mod data;
pub mod fetch;
pub mod layout;
pub mod render;
pub mod state;

#[cfg(test)]
mod test {
    use quickcheck::Arbitrary;
    use time::{Date, PrimitiveDateTime, Time};

    #[derive(Copy, Clone, Debug)]
    pub struct ArbitraryDateTime(pub PrimitiveDateTime);

    impl Arbitrary for ArbitraryDateTime {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let date = Date::from_ordinal_date(
                i32::arbitrary(g).clamp(0, 9999),
                u16::arbitrary(g).clamp(1, 365),
            )
            .unwrap();
            let time = Time::from_hms(
                u8::arbitrary(g).clamp(0, 23),
                u8::arbitrary(g).clamp(0, 59),
                u8::arbitrary(g).clamp(0, 59),
            )
            .unwrap();
            ArbitraryDateTime(PrimitiveDateTime::new(date, time))
        }
    }
}
