//! Bounded structural accounting through serde, without JSON formatting or a second AST walk.
use serde::{Serialize, Serializer, ser};

pub(super) fn retained(value: &impl Serialize, roots: usize, limit: usize) -> Option<usize> {
    let mut size = Size {
        bytes: roots,
        remaining: limit.checked_sub(roots)?,
    };
    value.serialize(&mut size).ok()?;
    Some(size.bytes)
}
struct Size {
    bytes: usize,
    remaining: usize,
}
impl Size {
    fn value(&mut self, payload: usize) -> Result<(), serde_json::Error> {
        // Reserve spare vector slots as well as enum capacity and tree links.
        const NODE_BYTES: usize =
            if std::mem::size_of::<crate::ast::Expr>() > std::mem::size_of::<sanim_core::Type>() {
                std::mem::size_of::<crate::ast::Expr>()
            } else {
                std::mem::size_of::<sanim_core::Type>()
            } * 2
                + 4 * std::mem::size_of::<usize>();
        // Compiler Strings grow geometrically; their capacity can exceed len.
        let bytes = payload
            .checked_mul(2)
            .and_then(|payload| NODE_BYTES.checked_add(payload))
            .ok_or_else(|| ser::Error::custom("compiler cache estimate overflow"))?;
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(|| ser::Error::custom("compiler cache byte budget reached"))?;
        self.bytes += bytes;
        Ok(())
    }
}
struct Compound<'a>(&'a mut Size);
macro_rules! scalars {
    ($($name:ident($ty:ty)),* $(,)?) => {$ (
        fn $name(self, _: $ty) -> Result<(), Self::Error> { self.value(0) }
    )*};
}
impl<'a> Serializer for &'a mut Size {
    type Ok = ();
    type Error = serde_json::Error;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;
    scalars!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char)
    );
    fn serialize_str(self, value: &str) -> Result<(), Self::Error> {
        self.value(value.len())
    }
    fn serialize_bytes(self, value: &[u8]) -> Result<(), Self::Error> {
        self.value(value.len())
    }
    fn serialize_none(self) -> Result<(), Self::Error> {
        self.value(0)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Self::Error> {
        self.value(0)?;
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Self::Error> {
        self.value(0)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Self::Error> {
        self.value(0)
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), Self::Error> {
        self.value(0)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.value(0)?;
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        self.value(0)?;
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.value(0)?;
        Ok(Compound(self))
    }
    fn serialize_tuple(self, _: usize) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(None)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(None)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(None)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(None)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(None)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Compound<'a>, Self::Error> {
        self.serialize_seq(None)
    }
}
macro_rules! sequences {
    ($($trait:ident::$method:ident),* $(,)?) => {$ (
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = serde_json::Error;
            fn $method<T:?Sized+Serialize>(&mut self,value:&T)->Result<(),Self::Error>{value.serialize(&mut *self.0)}
            fn end(self)->Result<(),Self::Error>{Ok(())}
        }
    )*};
}
sequences!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
    SerializeTupleVariant::serialize_field
);
impl ser::SerializeMap for Compound<'_> {
    type Ok = ();
    type Error = serde_json::Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut *self.0)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(&mut *self.0)
    }
    fn end(self) -> Result<(), Self::Error> {
        Ok(())
    }
}
macro_rules! structs {
    ($($trait:ident),* $(,)?) => {$ (
        impl ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = serde_json::Error;
            fn serialize_field<T:?Sized+Serialize>(&mut self,_:&'static str,value:&T)->Result<(),Self::Error>{value.serialize(&mut *self.0)}
            fn end(self)->Result<(),Self::Error>{Ok(())}
        }
    )*};
}
structs!(SerializeStruct, SerializeStructVariant);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tuple_map_keys_and_nested_payloads_are_bounded_without_json() {
        let value =
            std::collections::BTreeMap::from([((1usize, 2usize), vec!["hello".to_string(); 4])]);
        let bytes = retained(&value, 32, usize::MAX).unwrap();
        assert!(bytes > std::mem::size_of_val(&value) + 20);
        assert_eq!(retained(&value, 32, bytes), Some(bytes));
        assert_eq!(retained(&value, 32, bytes - 1), None);
        assert_eq!(retained(&value, usize::MAX, 100), None);
    }
}
