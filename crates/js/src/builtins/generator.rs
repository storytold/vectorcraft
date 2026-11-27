//! The generator built-ins (ECMA-262 11th edition, <https://262.ecma-international.org/11.0/>):
//! `%GeneratorFunction%` with its `prototype` (sections 25.2.1 and 25.2.3) and
//! `%GeneratorPrototype%` with `next`, `return` and `throw` (section 25.4.1), which inherits
//! from `%IteratorPrototype%`. Installed in a realm with [`crate::Options::es2015`] only, after the
//! iterators. `GeneratorFunction` is not a global; a script reaches it through a generator
//! function's `constructor`.

use super::{data, native, unreachable_native};
use crate::generator::ResumeMode;
use crate::heap::{CONFIGURABLE, Class, HIDDEN, NativeKind, ObjId, Object, Prop};
use crate::interp::{Interp, JsResult};
use crate::value::Value;

pub(super) fn install(interp: &mut Interp, function: ObjId) -> JsResult<()> {
    let Some(to_string_tag) = interp.well_known(|w| &w.to_string_tag) else { return Ok(()) };
    let Some(mut objects) = interp.symbols.objects else { return Ok(()) };
    let object_proto = interp.intrinsics.object_proto;
    let iterator_proto = interp.heap.object(objects.array_iterator_proto).and_then(|o| o.proto).unwrap_or(object_proto);

    let generator_proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(iterator_proto)));
    for (name, mode) in [("next", ResumeMode::Next), ("return", ResumeMode::Return), ("throw", ResumeMode::Throw)] {
        let m = native(interp, name, unreachable_native, 1, NativeKind::Resume(mode), false)?;
        data(interp, generator_proto, name, Value::Object(m), HIDDEN)?;
    }
    interp.define_sym(generator_proto, to_string_tag.clone(), Prop::Data { value: Value::str("Generator"), flags: CONFIGURABLE })?;

    let function_proto = interp.intrinsics.function_proto;
    // %GeneratorFunction.prototype% is an ordinary object, not a function, that inherits from
    // Function.prototype (section 25.2.3).
    let generator_function_proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(function_proto)));
    data(interp, generator_function_proto, "prototype", Value::Object(generator_proto), CONFIGURABLE)?;
    data(interp, generator_proto, "constructor", Value::Object(generator_function_proto), CONFIGURABLE)?;
    interp.define_sym(generator_function_proto, to_string_tag, Prop::Data { value: Value::str("GeneratorFunction"), flags: CONFIGURABLE })?;

    let generator_function = native(interp, "GeneratorFunction", generator_function_ctor, 1, NativeKind::Plain, true)?;
    if let Some(o) = interp.heap.object_mut(generator_function) {
        o.proto = Some(function);
    }
    data(interp, generator_function, "prototype", Value::Object(generator_function_proto), 0)?;
    data(interp, generator_function_proto, "constructor", Value::Object(generator_function), CONFIGURABLE)?;

    objects.generator_proto = generator_proto;
    objects.generator_function_proto = generator_function_proto;
    interp.symbols.objects = Some(objects);
    Ok(())
}

/// `GeneratorFunction(p1, …, body)` and `new GeneratorFunction(…)` (section 25.2.1.1).
fn generator_function_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    super::create_dynamic_function(interp, args, true)
}
