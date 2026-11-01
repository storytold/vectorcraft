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
