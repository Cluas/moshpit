//! A serde `Deserializer` over a parsed [`Value`] with `JSONDecoder`'s rules:
//! an integer field reads a number the way Swift's `Int` does
//! ([`Number::decoder_int`]), a floating one the way `Double` does, `null`
//! is an absent optional, and nothing converts between strings, numbers and
//! Booleans. A number token `JSONDecoder` would refuse to decode fails here
//! as it does there — only once a field asks for it.

use std::fmt;

use serde::de::{
    self, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, SeqAccess, Visitor,
};

use super::json::{Number, Object, Value};

/// Why a value is not the type asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl de::Error for Error {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

fn mismatch(expected: &str, value: &Value) -> Error {
    let found = match value {
        Value::Null => "null",
        Value::Bool(_) => "a Boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    };
    Error(format!("expected {expected}, found {found}"))
}

/// Deserializes from a borrowed tree.
pub struct ValueDeserializer<'a>(pub &'a Value);

fn int(value: &Value) -> Result<i64, Error> {
    match value {
        Value::Number(number) => number
            .decoder_int()
            .ok_or_else(|| Error(format!("number <{}> does not fit in Int", number.literal()))),
        other => Err(mismatch("an integer", other)),
    }
}

fn double(number: &Number) -> Result<f64, Error> {
    number
        .decoder_double()
        .ok_or_else(|| Error(format!("number <{}> is not a Double", number.literal())))
}

macro_rules! signed {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            let value = int(self.0)?;
            visitor
                .$visit(<$ty>::try_from(value).map_err(|_| Error(format!("{value} does not fit")))?)
        }
    };
}

macro_rules! unsigned {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            // UInt64 fields beyond Int64 would need their own rule; no model
            // has one.
            let value = int(self.0)?;
            visitor
                .$visit(<$ty>::try_from(value).map_err(|_| Error(format!("{value} does not fit")))?)
        }
    };
}

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(flag) => visitor.visit_bool(*flag),
            Value::Number(number) => match number.decoder_int() {
                Some(value) => visitor.visit_i64(value),
                None => visitor.visit_f64(double(number)?),
            },
            Value::String(text) => visitor.visit_borrowed_str(text),
            Value::Array(items) => visitor.visit_seq(Items(items.iter())),
            Value::Object(object) => visitor.visit_map(Members::new(object)),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Bool(flag) => visitor.visit_bool(*flag),
            other => Err(mismatch("a Boolean", other)),
        }
    }

    signed!(deserialize_i8, visit_i8, i8);
    signed!(deserialize_i16, visit_i16, i16);
    signed!(deserialize_i32, visit_i32, i32);
    signed!(deserialize_i64, visit_i64, i64);
    unsigned!(deserialize_u8, visit_u8, u8);
    unsigned!(deserialize_u16, visit_u16, u16);
    unsigned!(deserialize_u32, visit_u32, u32);
    unsigned!(deserialize_u64, visit_u64, u64);

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_f64(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Number(number) => visitor.visit_f64(double(number)?),
            other => Err(mismatch("a number", other)),
        }
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::String(text) => visitor.visit_borrowed_str(text),
            other => Err(mismatch("a string", other)),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_any(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_any(visitor)
    }

    /// `decodeIfPresent`: `null` is absent.
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            other => Err(mismatch("null", other)),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Array(items) => visitor.visit_seq(Items(items.iter())),
            other => Err(mismatch("an array", other)),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.0 {
            Value::Object(object) => visitor.visit_map(Members::new(object)),
            other => Err(mismatch("an object", other)),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_map(visitor)
    }

    /// A Swift enum with a `String` raw value.
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.0 {
            Value::String(text) => visitor.visit_enum(text.as_str().into_deserializer()),
            other => Err(mismatch("a string", other)),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }
}

struct Items<'a>(std::slice::Iter<'a, Value>);

impl<'de> SeqAccess<'de> for Items<'de> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        self.0
            .next()
            .map(|item| seed.deserialize(ValueDeserializer(item)))
            .transpose()
    }
}

struct Members<'a> {
    members: Vec<(&'a str, &'a Value)>,
    next: usize,
}

impl<'a> Members<'a> {
    fn new(object: &'a Object) -> Self {
        Self {
            members: object.iter().collect(),
            next: 0,
        }
    }
}

impl<'de> MapAccess<'de> for Members<'de> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        let Some(&(key, _)) = self.members.get(self.next) else {
            return Ok(None);
        };
        seed.deserialize(de::value::BorrowedStrDeserializer::new(key))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let (_, value) = self.members[self.next];
        self.next += 1;
        seed.deserialize(ValueDeserializer(value))
    }
}
