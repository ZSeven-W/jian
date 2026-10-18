//! A false ternary branch must leave exactly one value on the VM stack.

use jian_core::expression::bytecode::{Chunk, OpCode};
use jian_core::expression::{chunks_to_snapshot, snapshot_to_chunks, Expression};
use jian_core::Runtime;
use jian_ops_schema::pack::ExpressionsSnapshot;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const PRICE: &str = "`¥${$app.capacity === '600ml' ? 159 : 129}`";

fn evaluate(source: &str, capacity: &str) -> Value {
    let rt = Runtime::new();
    rt.state.app_set("capacity", json!(capacity));
    let (value, warnings) = Expression::compile(source)
        .unwrap()
        .eval(&rt.state, None, None);
    assert!(warnings.is_empty(), "{source}: {warnings:?}");
    value.0
}

#[test]
fn retained_product_price_template_has_no_condition_prefix() {
    assert_eq!(evaluate(PRICE, "450ml"), json!("¥129"));
    assert_eq!(evaluate(PRICE, "600ml"), json!("¥159"));
    assert_eq!(
        evaluate("`已选：${false ? '600ml' : '450ml'} · ${2}`", "450ml"),
        json!("已选：450ml · 2")
    );
}

#[test]
fn false_ternaries_preserve_surrounding_arithmetic_arrays_objects_and_calls() {
    for (source, want) in [
        ("100 + (false ? 3 : 7)", json!(107)),
        ("[11, false ? 1 : 2, 33]", json!([11, 2, 33])),
        (
            "{amount: false ? 159 : 129, quantity: 1}",
            json!({"amount":129,"quantity":1}),
        ),
        ("min(100, false ? 3 : 7)", json!(7)),
        ("10 + (false ? 1 : (false ? 2 : 3))", json!(13)),
        ("10 + (true ? (false ? 2 : 3) : 4)", json!(13)),
        ("false ? missing_builtin() : 2", json!(2)),
        ("true ? 1 : missing_builtin()", json!(1)),
    ] {
        assert_eq!(evaluate(source, "450ml"), want, "{source}");
    }
}

#[test]
fn short_circuit_operands_keep_their_existing_values() {
    for (source, want) in [
        ("false && missing_builtin()", json!(false)),
        ("true || missing_builtin()", json!(true)),
        ("0 && 7", json!(0)),
        ("'' || 'fallback'", json!("fallback")),
        ("`value=${false && 7}`", json!("value=false")),
        ("[1, false || (false ? 3 : 7), 9]", json!([1, 7, 9])),
        ("[1, true && (false ? 3 : 7), 9]", json!([1, 7, 9])),
    ] {
        assert_eq!(evaluate(source, "450ml"), want, "{source}");
    }
}

#[test]
fn legacy_precompiled_price_uses_the_corrected_compiler_on_load() {
    // Exact opcode shape emitted before the fix: JumpIfFalse retained `false`
    // below 129, so TemplateAppend produced false129 and lost the currency.
    let old = Chunk {
        ops: vec![
            OpCode::PushString(0),
            OpCode::PushString(1),
            OpCode::TemplateAppend,
            OpCode::PushScopeRef(0),
            OpCode::PushString(2),
            OpCode::EqStrict,
            OpCode::JumpIfFalse(2),
            OpCode::PushNum(159.0),
            OpCode::Jump(1),
            OpCode::PushNum(129.0),
            OpCode::TemplateAppend,
            OpCode::Return,
        ],
        strings: vec!["".into(), "¥".into(), "600ml".into()],
        scope_paths: vec!["$app.capacity".into()],
    };
    let old_snapshot = chunks_to_snapshot(&BTreeMap::from([(PRICE.into(), old)]));
    let bytes = old_snapshot.write_bytes().unwrap();
    let decoded = ExpressionsSnapshot::read_bytes(&bytes).unwrap();
    let restored = snapshot_to_chunks(&decoded);
    let rt = Runtime::new();
    for (capacity, want) in [("450ml", "¥129"), ("600ml", "¥159")] {
        rt.state.app_set("capacity", json!(capacity));
        let expression = Expression {
            source: PRICE.into(),
            chunk: restored[PRICE].clone(),
        };
        let (value, warnings) = expression.eval(&rt.state, None, None);
        assert!(warnings.is_empty());
        assert_eq!(value.0, json!(want));
    }
}

#[test]
fn newly_compiled_conditionals_survive_the_existing_pack_format() {
    let expression = Expression::compile(PRICE).unwrap();
    let snapshot = chunks_to_snapshot(&BTreeMap::from([(PRICE.into(), expression.chunk.clone())]));
    let bytes = snapshot.write_bytes().unwrap();
    let decoded = ExpressionsSnapshot::read_bytes(&bytes).unwrap();
    let restored = snapshot_to_chunks(&decoded);
    assert_eq!(restored[PRICE].ops, expression.chunk.ops);
}
