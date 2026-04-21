//! Stack-machine VM for compiled expressions.

use super::bytecode::{Chunk, OpCode};
use super::diag::{DiagKind, Diagnostic, Span};
use crate::value::RuntimeValue;
use serde_json::{json, Value};

pub trait EvalContext {
    /// Resolve `$scope.a.b.c` where `path` is the literal source string.
    fn lookup_scope(&self, path: &str) -> Option<RuntimeValue>;

    /// Invoke a builtin function. Returns `None` if not found.
    fn call_builtin(
        &self,
        name: &str,
        args: &[RuntimeValue],
    ) -> Option<Result<RuntimeValue, Diagnostic>>;

    /// Push a warning diagnostic.
    fn warn(&self, _diag: Diagnostic) {}

    /// Optional cache for HOF sub-expressions. Default: no cache.
    fn cache(&self) -> Option<&super::cache::ExpressionCache> {
        None
    }
}

pub fn run(chunk: &Chunk, ctx: &dyn EvalContext) -> Result<RuntimeValue, Diagnostic> {
    let mut stack: Vec<RuntimeValue> = Vec::with_capacity(16);
    let mut ip: usize = 0;
    let ops = &chunk.ops;
    while ip < ops.len() {
        let op = &ops[ip];
        match op {
            OpCode::PushNum(n) => stack.push(RuntimeValue::from_f64(*n)),
            OpCode::PushBool(b) => stack.push(RuntimeValue::from_bool(*b)),
            OpCode::PushNull => stack.push(RuntimeValue::null()),
            OpCode::PushString(i) => {
                let s = chunk
                    .strings
                    .get(*i as usize)
                    .ok_or_else(|| vm_bug("string pool oob", ip))?;
                stack.push(RuntimeValue::from_string(s.clone()));
            }
            OpCode::PushScopeRef(i) => {
                let path = chunk
                    .scope_paths
                    .get(*i as usize)
                    .ok_or_else(|| vm_bug("scope pool oob", ip))?;
                let v = ctx.lookup_scope(path).unwrap_or_else(|| {
                    ctx.warn(Diagnostic {
                        kind: DiagKind::UnknownIdentifier,
                        message: format!("unknown scope path `{}`", path),
                        span: Span::zero(),
                    });
                    RuntimeValue::null()
                });
                stack.push(v);
            }
            OpCode::MemberGet(i) => {
                let obj = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let name = chunk
                    .strings
                    .get(*i as usize)
                    .ok_or_else(|| vm_bug("string pool oob", ip))?;
                stack.push(member_get(&obj, name));
            }
            OpCode::IndexGet => {
                let idx = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let obj = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                stack.push(index_get(&obj, &idx));
            }
            OpCode::Not => {
                let v = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                stack.push(RuntimeValue::from_bool(!truthy(&v)));
            }
            OpCode::Negate => {
                let v = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let n = coerce_number(&v).unwrap_or(0.0);
                stack.push(RuntimeValue::from_f64(-n));
            }
            OpCode::UnaryPlus => {
                let v = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let n = coerce_number(&v).unwrap_or(0.0);
                stack.push(RuntimeValue::from_f64(n));
            }
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | OpCode::Mod => {
                let r = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let l = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                stack.push(arith(op, &l, &r));
            }
            OpCode::Eq
            | OpCode::NotEq
            | OpCode::EqStrict
            | OpCode::NotEqStrict
            | OpCode::Lt
            | OpCode::Gt
            | OpCode::LtEq
            | OpCode::GtEq => {
                let r = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let l = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                stack.push(compare(op, &l, &r));
            }
            OpCode::JumpIfFalse(off) => {
                let v = stack.last().cloned().unwrap_or_else(RuntimeValue::null);
                if !truthy(&v) {
                    ip = ((ip as i32) + *off + 1) as usize;
                    continue;
                } else {
                    stack.pop();
                }
            }
            OpCode::JumpIfTrue(off) => {
                let v = stack.last().cloned().unwrap_or_else(RuntimeValue::null);
                if truthy(&v) {
                    ip = ((ip as i32) + *off + 1) as usize;
                    continue;
                } else {
                    stack.pop();
                }
            }
            OpCode::Jump(off) => {
                ip = ((ip as i32) + *off + 1) as usize;
                continue;
            }
            OpCode::NullCoalesce => {
                let r = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let l = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                stack.push(if l.is_null() { r } else { l });
            }
            OpCode::MakeArray(n) => {
                let n = *n as usize;
                let at = stack.len() - n;
                let items: Vec<Value> = stack.drain(at..).map(|v| v.0).collect();
                stack.push(RuntimeValue(Value::Array(items)));
            }
            OpCode::PushObjectKey(i) => {
                let k = chunk
                    .strings
                    .get(*i as usize)
                    .ok_or_else(|| vm_bug("string pool oob", ip))?;
                stack.push(RuntimeValue(json!({ "__objkey__": k })));
            }
            OpCode::MakeObject(n) => {
                let n = *n as usize;
                let take = n * 2;
                let at = stack.len() - take;
                let drained: Vec<RuntimeValue> = stack.drain(at..).collect();
                let mut map = serde_json::Map::new();
                let mut i = 0;
                while i < drained.len() {
                    let k = drained[i]
                        .0
                        .get("__objkey__")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| vm_bug("expected __objkey__ marker", ip))?
                        .to_owned();
                    let v = drained[i + 1].0.clone();
                    map.insert(k, v);
                    i += 2;
                }
                stack.push(RuntimeValue(Value::Object(map)));
            }
            OpCode::TemplateAppend => {
                let r = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let l = stack.pop().ok_or_else(|| vm_bug("stack underflow", ip))?;
                let mut s = to_display(&l);
                s.push_str(&to_display(&r));
                stack.push(RuntimeValue::from_string(s));
            }
            OpCode::CallBuiltin(name_i, argc) => {
                let name = chunk
                    .strings
                    .get(*name_i as usize)
                    .ok_or_else(|| vm_bug("string pool oob", ip))?;
                let argc = *argc as usize;
                let at = stack.len() - argc;
                let args: Vec<RuntimeValue> = stack.drain(at..).collect();
                match ctx.call_builtin(name, &args) {
                    Some(Ok(v)) => stack.push(v),
                    Some(Err(d)) => {
                        ctx.warn(d);
                        stack.push(RuntimeValue::null());
                    }
                    None => {
                        ctx.warn(Diagnostic {
                            kind: DiagKind::UnknownFunction,
                            message: format!("unknown function `{}`", name),
                            span: Span::zero(),
                        });
                        stack.push(RuntimeValue::null());
                    }
                }
            }
            OpCode::Return => {
                return Ok(stack.pop().unwrap_or_else(RuntimeValue::null));
            }
        }
        ip += 1;
    }
    Ok(stack.pop().unwrap_or_else(RuntimeValue::null))
}

fn vm_bug(msg: &str, ip: usize) -> Diagnostic {
    Diagnostic {
        kind: DiagKind::CompileError,
        message: format!("VM bug at ip={}: {}", ip, msg),
        span: Span::zero(),
    }
}

fn truthy(v: &RuntimeValue) -> bool {
    match &v.0 {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn coerce_number(v: &RuntimeValue) -> Option<f64> {
    match &v.0 {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Null => Some(0.0),
        _ => None,
    }
}

fn to_display(v: &RuntimeValue) -> String {
    match &v.0 {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e17 => {
                format!("{}", f as i64)
            }
            Some(f) => format!("{}", f),
            None => n.to_string(),
        },
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn member_get(obj: &RuntimeValue, name: &str) -> RuntimeValue {
    match &obj.0 {
        Value::Object(m) => m
            .get(name)
            .cloned()
            .map(RuntimeValue)
            .unwrap_or_else(RuntimeValue::null),
        _ => RuntimeValue::null(),
    }
}

fn index_get(obj: &RuntimeValue, idx: &RuntimeValue) -> RuntimeValue {
    match (&obj.0, &idx.0) {
        (Value::Array(a), Value::Number(n)) => {
            let i = n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .unwrap_or(-1);
            if i < 0 {
                return RuntimeValue::null();
            }
            a.get(i as usize)
                .cloned()
                .map(RuntimeValue)
                .unwrap_or_else(RuntimeValue::null)
        }
        (Value::Object(m), Value::String(k)) => m
            .get(k)
            .cloned()
            .map(RuntimeValue)
            .unwrap_or_else(RuntimeValue::null),
        _ => RuntimeValue::null(),
    }
}

fn arith(op: &OpCode, l: &RuntimeValue, r: &RuntimeValue) -> RuntimeValue {
    if matches!(op, OpCode::Add) {
        if matches!(l.0, Value::String(_)) || matches!(r.0, Value::String(_)) {
            return RuntimeValue::from_string(format!("{}{}", to_display(l), to_display(r)));
        }
        if let (Value::Array(la), Value::Array(ra)) = (&l.0, &r.0) {
            let mut out = la.clone();
            out.extend(ra.clone());
            return RuntimeValue(Value::Array(out));
        }
    }
    let lv = coerce_number(l).unwrap_or(f64::NAN);
    let rv = coerce_number(r).unwrap_or(f64::NAN);
    let result = match op {
        OpCode::Add => lv + rv,
        OpCode::Sub => lv - rv,
        OpCode::Mul => lv * rv,
        OpCode::Div => {
            if rv == 0.0 {
                return RuntimeValue::null();
            }
            lv / rv
        }
        OpCode::Mod => {
            if rv == 0.0 {
                return RuntimeValue::null();
            }
            lv % rv
        }
        _ => unreachable!(),
    };
    if result.is_nan() {
        RuntimeValue::null()
    } else {
        RuntimeValue::from_f64(result)
    }
}

fn compare(op: &OpCode, l: &RuntimeValue, r: &RuntimeValue) -> RuntimeValue {
    let b = match op {
        OpCode::EqStrict => l == r,
        OpCode::NotEqStrict => l != r,
        OpCode::Eq => l.loose_eq(r),
        OpCode::NotEq => !l.loose_eq(r),
        OpCode::Lt | OpCode::Gt | OpCode::LtEq | OpCode::GtEq => {
            let la = coerce_number(l);
            let ra = coerce_number(r);
            match (la, ra) {
                (Some(a), Some(b)) => match op {
                    OpCode::Lt => a < b,
                    OpCode::Gt => a > b,
                    OpCode::LtEq => a <= b,
                    OpCode::GtEq => a >= b,
                    _ => unreachable!(),
                },
                _ => false,
            }
        }
        _ => false,
    };
    RuntimeValue::from_bool(b)
}

#[cfg(test)]
mod tests {
    use super::super::{compiler::compile, parser::parse};
    use super::*;

    struct EmptyCtx;
    impl EvalContext for EmptyCtx {
        fn lookup_scope(&self, _: &str) -> Option<RuntimeValue> {
            None
        }
        fn call_builtin(
            &self,
            _: &str,
            _: &[RuntimeValue],
        ) -> Option<Result<RuntimeValue, Diagnostic>> {
            None
        }
    }

    fn eval(src: &str) -> RuntimeValue {
        let chunk = compile(&parse(src).unwrap()).unwrap();
        run(&chunk, &EmptyCtx).unwrap()
    }

    #[test]
    fn number_literal() {
        assert_eq!(eval("42").as_f64(), Some(42.0));
    }
    #[test]
    fn add() {
        assert_eq!(eval("1 + 2").as_f64(), Some(3.0));
    }
    #[test]
    fn sub() {
        assert_eq!(eval("10 - 3").as_f64(), Some(7.0));
    }
    #[test]
    fn mul() {
        assert_eq!(eval("4 * 5").as_f64(), Some(20.0));
    }
    #[test]
    fn div() {
        assert_eq!(eval("20 / 4").as_f64(), Some(5.0));
    }
    #[test]
    fn div_by_zero_null() {
        assert!(eval("1 / 0").is_null());
    }
    #[test]
    fn mod_op() {
        assert_eq!(eval("10 % 3").as_f64(), Some(1.0));
    }
    #[test]
    fn precedence() {
        assert_eq!(eval("1 + 2 * 3").as_f64(), Some(7.0));
    }
    #[test]
    fn string_concat() {
        assert_eq!(
            eval("\"Hello, \" + \"world\"").as_str(),
            Some("Hello, world")
        );
    }
    #[test]
    fn comparison() {
        assert_eq!(eval("5 > 3").as_bool(), Some(true));
        assert_eq!(eval("5 == 5").as_bool(), Some(true));
        assert_eq!(eval("5 !== \"5\"").as_bool(), Some(true));
    }
    #[test]
    fn logical_and_short_circuits() {
        assert_eq!(eval("false && (1 / 0)").as_bool(), Some(false));
    }
    #[test]
    fn logical_or_returns_value() {
        assert_eq!(eval("null || \"fallback\"").as_str(), Some("fallback"));
    }
    #[test]
    fn nullish_coalesce() {
        assert_eq!(eval("null ?? 42").as_f64(), Some(42.0));
        assert_eq!(eval("0 ?? 42").as_f64(), Some(0.0));
    }
    #[test]
    fn ternary() {
        assert_eq!(eval("true ? 1 : 2").as_f64(), Some(1.0));
    }
    #[test]
    fn array_and_index() {
        assert_eq!(eval("[10, 20, 30][1]").as_f64(), Some(20.0));
    }
    #[test]
    fn object_and_member() {
        assert_eq!(eval("{a: 1, b: 2}.b").as_f64(), Some(2.0));
    }
    #[test]
    fn template_roundtrip() {
        assert_eq!(eval("`sum = ${1 + 2}`").as_str(), Some("sum = 3"));
    }
}
