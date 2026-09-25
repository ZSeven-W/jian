//! The concrete buffer the `PenNode` decoder reads node fields into.
//!
//! It is `serde_json::Value` without the per-object `IndexMap`: objects are
//! plain ordered pair lists (repeated keys kept, as serde's own buffering
//! does), which makes buffering a node about as cheap as the derive's private
//! `Content` buffer was. As a `Deserializer` it mirrors `serde_json::Value`'s
//! impl method for method — same visitor calls, same error messages — so
//! field types decode exactly as they would from a `Value`.

use serde::de::value::StringDeserializer;
use serde::de::{
    Deserialize, DeserializeSeed, Deserializer, EnumAccess, Error as _, IntoDeserializer,
    MapAccess, SeqAccess, Unexpected, VariantAccess, Visitor,
};
use serde_json::Error;
use std::fmt;
use std::vec;

pub(super) enum Buf {
    Null,
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    Str(String),
    Seq(Vec<Buf>),
    Map(Vec<(String, Buf)>),
}

impl Buf {
    pub(super) fn unexpected(&self) -> Unexpected<'_> {
        match self {
            Buf::Null => Unexpected::Unit,
            Buf::Bool(b) => Unexpected::Bool(*b),
            Buf::U64(n) => Unexpected::Unsigned(*n),
            Buf::I64(n) => Unexpected::Signed(*n),
            Buf::F64(n) => Unexpected::Float(*n),
            Buf::Str(s) => Unexpected::Str(s),
            Buf::Seq(_) => Unexpected::Seq,
            Buf::Map(_) => Unexpected::Map,
        }
    }

    fn invalid_type(&self, exp: &dyn serde::de::Expected) -> Error {
        Error::invalid_type(self.unexpected(), exp)
    }
}

impl<'de> Deserialize<'de> for Buf {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(BufVisitor)
    }
}

/// Accepts exactly what `serde_json::Value`'s visitor accepts from JSON.
struct BufVisitor;

impl<'de> Visitor<'de> for BufVisitor {
    type Value = Buf;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Buf, E> {
        Ok(Buf::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Buf, E> {
        // `serde_json::Number` stores every non-negative integer as unsigned.
        Ok(u64::try_from(v).map_or(Buf::I64(v), Buf::U64))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Buf, E> {
        Ok(Buf::U64(v))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Buf, E> {
        // Like `Value`, a non-finite float becomes null.
        Ok(if v.is_finite() {
            Buf::F64(v)
        } else {
            Buf::Null
        })
    }

    fn visit_str<E>(self, v: &str) -> Result<Buf, E> {
        Ok(Buf::Str(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Buf, E> {
        Ok(Buf::Str(v))
    }

    fn visit_none<E>(self) -> Result<Buf, E> {
        Ok(Buf::Null)
    }

    fn visit_unit<E>(self) -> Result<Buf, E> {
        Ok(Buf::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Buf, D::Error> {
        Buf::deserialize(d)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Buf, A::Error> {
        let mut items = Vec::with_capacity(access.size_hint().unwrap_or(0).min(4096));
        while let Some(item) = access.next_element()? {
            items.push(item);
        }
        Ok(Buf::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Buf, A::Error> {
        let mut entries = Vec::with_capacity(access.size_hint().unwrap_or(0).min(4096));
        while let Some(key) = access.next_key::<String>()? {
            entries.push((key, access.next_value()?));
        }
        Ok(Buf::Map(entries))
    }
}

pub(super) fn visit_seq<'de, V: Visitor<'de>>(
    items: Vec<Buf>,
    visitor: V,
) -> Result<V::Value, Error> {
    let len = items.len();
    let mut access = SeqBuf(items.into_iter());
    let value = visitor.visit_seq(&mut access)?;
    if access.0.len() == 0 {
        Ok(value)
    } else {
        Err(Error::invalid_length(len, &"fewer elements in array"))
    }
}

pub(super) fn visit_map<'de, V: Visitor<'de>>(
    entries: Vec<(String, Buf)>,
    visitor: V,
) -> Result<V::Value, Error> {
    let len = entries.len();
    let mut access = MapBuf {
        iter: entries.into_iter(),
        value: None,
    };
    let value = visitor.visit_map(&mut access)?;
    if access.iter.len() == 0 {
        Ok(value)
    } else {
        Err(Error::invalid_length(len, &"fewer elements in map"))
    }
}

impl<'de> Deserializer<'de> for Buf {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Null => visitor.visit_unit(),
            Buf::Bool(v) => visitor.visit_bool(v),
            Buf::U64(v) => visitor.visit_u64(v),
            Buf::I64(v) => visitor.visit_i64(v),
            Buf::F64(v) => visitor.visit_f64(v),
            Buf::Str(v) => visitor.visit_string(v),
            Buf::Seq(v) => visit_seq(v, visitor),
            Buf::Map(v) => visit_map(v, visitor),
        }
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_number(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self {
            Buf::Map(entries) => {
                let mut iter = entries.into_iter();
                let single = "map with a single key";
                let (variant, value) = iter
                    .next()
                    .ok_or_else(|| Error::invalid_value(Unexpected::Map, &single))?;
                if iter.next().is_some() {
                    return Err(Error::invalid_value(Unexpected::Map, &single));
                }
                visitor.visit_enum(EnumBuf {
                    variant,
                    value: Some(value),
                })
            }
            Buf::Str(variant) => visitor.visit_enum(EnumBuf {
                variant,
                value: None,
            }),
            other => Err(other.invalid_type(&"string or map")),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Bool(v) => visitor.visit_bool(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_string(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_string(visitor)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Str(v) => visitor.visit_string(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_byte_buf(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Str(v) => visitor.visit_string(v),
            Buf::Seq(v) => visit_seq(v, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Null => visitor.visit_unit(),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Seq(v) => visit_seq(v, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::Map(v) => visit_map(v, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self {
            Buf::Seq(v) => visit_seq(v, visitor),
            Buf::Map(v) => visit_map(v, visitor),
            _ => Err(self.invalid_type(&visitor)),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_string(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        drop(self);
        visitor.visit_unit()
    }
}

impl Buf {
    fn deserialize_number<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Buf::U64(v) => visitor.visit_u64(v),
            Buf::I64(v) => visitor.visit_i64(v),
            Buf::F64(v) => visitor.visit_f64(v),
            _ => Err(self.invalid_type(&visitor)),
        }
    }
}

impl<'de> IntoDeserializer<'de, Error> for Buf {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

struct SeqBuf(vec::IntoIter<Buf>);

impl<'de> SeqAccess<'de> for SeqBuf {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        self.0.next().map(|item| seed.deserialize(item)).transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        match self.0.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

struct MapBuf {
    iter: vec::IntoIter<(String, Buf)>,
    value: Option<Buf>,
}

impl<'de> MapAccess<'de> for MapBuf {
    type Error = Error;

    fn next_key_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        match self.iter.next() {
            Some((key, value)) => {
                self.value = Some(value);
                let key: StringDeserializer<Error> = key.into_deserializer();
                seed.deserialize(key).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, Error> {
        match self.value.take() {
            Some(value) => seed.deserialize(value),
            None => Err(Error::custom("value is missing")),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        match self.iter.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

struct EnumBuf {
    variant: String,
    value: Option<Buf>,
}

impl<'de> EnumAccess<'de> for EnumBuf {
    type Error = Error;
    type Variant = VariantBuf;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, VariantBuf), Error> {
        let variant: StringDeserializer<Error> = self.variant.into_deserializer();
        let rest = VariantBuf { value: self.value };
        seed.deserialize(variant).map(|v| (v, rest))
    }
}

struct VariantBuf {
    value: Option<Buf>,
}

impl<'de> VariantAccess<'de> for VariantBuf {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        match self.value {
            Some(value) => Deserialize::deserialize(value),
            None => Ok(()),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        match self.value {
            Some(value) => seed.deserialize(value),
            None => Err(Error::invalid_type(
                Unexpected::UnitVariant,
                &"newtype variant",
            )),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Error> {
        match self.value {
            Some(Buf::Seq(v)) if v.is_empty() => visitor.visit_unit(),
            Some(Buf::Seq(v)) => visit_seq(v, visitor),
            Some(other) => Err(other.invalid_type(&"tuple variant")),
            None => Err(Error::invalid_type(
                Unexpected::UnitVariant,
                &"tuple variant",
            )),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.value {
            Some(Buf::Map(v)) => visit_map(v, visitor),
            Some(other) => Err(other.invalid_type(&"struct variant")),
            None => Err(Error::invalid_type(
                Unexpected::UnitVariant,
                &"struct variant",
            )),
        }
    }
}
