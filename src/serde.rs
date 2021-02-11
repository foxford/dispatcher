use std::ops::Bound;

use chrono::{DateTime, Utc};

type BoundedDatetimeTuple = (Bound<DateTime<Utc>>, Bound<DateTime<Utc>>);

pub(crate) mod ts_seconds_bound_tuple {
    use std::fmt;
    use std::ops::Bound;

    use super::BoundedDatetimeTuple;
    use chrono::{DateTime, NaiveDateTime, Utc};
    use serde::{de, ser};

    pub(crate) fn serialize<S>(
        value: &(Bound<DateTime<Utc>>, Bound<DateTime<Utc>>),
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        use ser::SerializeTuple;

        let (lt, rt) = value;
        let mut tup = serializer.serialize_tuple(2)?;

        match lt {
            Bound::Included(lt) => {
                let val = lt.timestamp();
                tup.serialize_element(&val)?;
            }
            Bound::Excluded(lt) => {
                // Adjusting the range to '[lt, rt)'
                let val = lt.timestamp() + 1;
                tup.serialize_element(&val)?;
            }
            Bound::Unbounded => {
                let val: Option<i64> = None;
                tup.serialize_element(&val)?;
            }
        }

        match rt {
            Bound::Included(rt) => {
                // Adjusting the range to '[lt, rt)'
                let val = rt.timestamp() - 1;
                tup.serialize_element(&val)?;
            }
            Bound::Excluded(rt) => {
                let val = rt.timestamp();
                tup.serialize_element(&val)?;
            }
            Bound::Unbounded => {
                let val: Option<i64> = None;
                tup.serialize_element(&val)?;
            }
        }

        tup.end()
    }

    pub fn deserialize<'de, D>(d: D) -> Result<BoundedDatetimeTuple, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_tuple(2, TupleSecondsTimestampVisitor)
    }

    struct TupleSecondsTimestampVisitor;

    impl<'de> de::Visitor<'de> for TupleSecondsTimestampVisitor {
        type Value = BoundedDatetimeTuple;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a [lt, rt) range of unix time (seconds) or null (unbounded)")
        }

        /// Deserialize a tuple of two Bounded DateTime<Utc>
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let lt = match seq.next_element()? {
                Some(Some(val)) => {
                    let dt = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(val, 0), Utc);
                    Bound::Included(dt)
                }
                Some(None) => Bound::Unbounded,
                None => return Err(de::Error::invalid_length(1, &self)),
            };

            let rt = match seq.next_element()? {
                Some(Some(val)) => {
                    let dt = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(val, 0), Utc);
                    Bound::Excluded(dt)
                }
                Some(None) => Bound::Unbounded,
                None => return Err(de::Error::invalid_length(2, &self)),
            };

            Ok((lt, rt))
        }
    }
}

///////////////////////////////////////////////////////////////////////////////

pub(crate) mod ts_seconds_option_bound_tuple {
    use serde::de;
    use std::fmt;

    use super::BoundedDatetimeTuple;

    pub fn deserialize<'de, D>(d: D) -> Result<Option<BoundedDatetimeTuple>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_option(TupleSecondsTimestampVisitor)
    }

    pub struct TupleSecondsTimestampVisitor;

    impl<'de> de::Visitor<'de> for TupleSecondsTimestampVisitor {
        type Value = Option<BoundedDatetimeTuple>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter
                .write_str("none or a [lt, rt) range of unix time (seconds) or null (unbounded)")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, d: D) -> Result<Self::Value, D::Error>
        where
            D: de::Deserializer<'de>,
        {
            let interval = super::ts_seconds_bound_tuple::deserialize(d)?;
            Ok(Some(interval))
        }
    }
}

//////////////////////////////////////////////////////////////////////////////

pub(crate) mod duration_seconds {
    use std::fmt;

    use chrono::Duration;
    use serde::de;

    pub fn deserialize<'de, D>(d: D) -> Result<Duration, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        d.deserialize_u64(SecondsDurationVisitor)
    }

    pub struct SecondsDurationVisitor;

    impl<'de> de::Visitor<'de> for SecondsDurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("duration (seconds)")
        }

        fn visit_u64<E>(self, seconds: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Duration::seconds(seconds as i64))
        }
    }
}
