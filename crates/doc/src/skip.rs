pub fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
pub fn is_true(v: &bool) -> bool {
    *v
}
pub fn is_one(v: &f32) -> bool {
    *v == 1.0
}
pub fn is_empty_vec<T>(v: &[T]) -> bool {
    v.is_empty()
}
pub fn is_empty_map(v: &serde_json::Map<String, serde_json::Value>) -> bool {
    v.is_empty()
}
/// Read a field's value and drop it, giving `None`: for state that is never taken from a file.
pub fn read_as_none<'de, D: serde::Deserializer<'de>, T>(d: D) -> Result<Option<T>, D::Error> {
    <serde::de::IgnoredAny as serde::Deserialize>::deserialize(d)?;
    Ok(None)
}
