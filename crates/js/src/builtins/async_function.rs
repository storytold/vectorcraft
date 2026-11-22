//! `AsyncFunction` and its prototype (ECMA-262 2017 section 25.5,
//! <https://262.ecma-international.org/8.0/#sec-async-function-objects>), in realms with
//! [`crate::Options::es2015`]: the prototype of every async function, and the constructor a script
//! reaches through it. `AsyncFunction` is not a global.

use super::{data, dynamic_function, native};
use crate::heap::{CONFIGURABLE, Class, NativeKind, ObjId, Object};
use crate::interp::{Interp, JsResult};
use crate::value::Value;

/// Creates `AsyncFunction` and `AsyncFunction.prototype`, keeping the prototype for the async
/// functions the realm makes.
pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let function_proto = interp.intrinsics.function_proto;
    let proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(function_proto)));
    interp.promises.async_function_proto = Some(proto);
    let ctor = native(interp, "AsyncFunction", async_function_ctor, 1, NativeKind::Plain, true)?;
    interp.promises.async_function_ctor = Some(ctor);
    // `Function` is the constructor's prototype (section 25.5.2).
    let function_ctor = constructor_of(interp, function_proto);
    if let Some(o) = interp.heap.object_mut(ctor) {
        o.proto = function_ctor.or(o.proto);
    }
    data(interp, ctor, "prototype", Value::Object(proto), 0)?;
    // Configurable and not writable (section 25.5.3.1), unlike the `constructor` of the ordinary
    // built-in prototypes.
    data(interp, proto, "constructor", Value::Object(ctor), CONFIGURABLE)
}

/// The `constructor` property `proto` has from the built-ins, when it is an object.
fn constructor_of(interp: &Interp, proto: ObjId) -> Option<ObjId> {
    match interp.get_own_property(proto, &crate::heap::key("constructor")) {
        Ok(Some(crate::heap::Prop::Data { value: Value::Object(o), .. })) => Some(o),
        _ => None,
    }
}

/// `AsyncFunction(p1, …, body)` and `new AsyncFunction(…)` (section 25.5.1.1).
fn async_function_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    dynamic_function(interp, args, true)
}
