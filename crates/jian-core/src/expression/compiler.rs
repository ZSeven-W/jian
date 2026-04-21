//! Translate AST to bytecode.

use super::ast::{BinaryOp, Expr, ExprKind, TemplatePart, UnaryOp};
use super::bytecode::{Chunk, OpCode};
use super::diag::Diagnostic;

pub fn compile(e: &Expr) -> Result<Chunk, Diagnostic> {
    let mut c = Chunk::new();
    emit(&mut c, e)?;
    c.push(OpCode::Return);
    Ok(c)
}

fn emit(c: &mut Chunk, e: &Expr) -> Result<(), Diagnostic> {
    match &e.kind {
        ExprKind::Number(n) => {
            c.push(OpCode::PushNum(*n));
        }
        ExprKind::Bool(b) => {
            c.push(OpCode::PushBool(*b));
        }
        ExprKind::Null => {
            c.push(OpCode::PushNull);
        }
        ExprKind::String(s) => {
            let i = c.intern_string(s);
            c.push(OpCode::PushString(i));
        }

        ExprKind::Identifier(name) => {
            // Bare identifier resolves to $self.<name> at runtime.
            let path = format!("$self.{}", name);
            let i = c.intern_scope_path(&path);
            c.push(OpCode::PushScopeRef(i));
        }

        ExprKind::ScopeRef(name, _access) => {
            let i = c.intern_scope_path(name);
            c.push(OpCode::PushScopeRef(i));
        }

        ExprKind::Member(inner, name) => {
            emit(c, inner)?;
            let i = c.intern_string(name);
            c.push(OpCode::MemberGet(i));
        }
        ExprKind::Index(inner, idx) => {
            emit(c, inner)?;
            emit(c, idx)?;
            c.push(OpCode::IndexGet);
        }

        ExprKind::Unary(op, inner) => {
            emit(c, inner)?;
            match op {
                UnaryOp::Not => c.push(OpCode::Not),
                UnaryOp::Neg => c.push(OpCode::Negate),
                UnaryOp::Pos => c.push(OpCode::UnaryPlus),
            };
        }

        ExprKind::Binary(op, l, r) => match op {
            BinaryOp::And => {
                emit(c, l)?;
                let jmp_short = c.push(OpCode::JumpIfFalse(0));
                emit(c, r)?;
                c.patch_jump(jmp_short, c.ops.len());
            }
            BinaryOp::Or => {
                emit(c, l)?;
                let jmp_short = c.push(OpCode::JumpIfTrue(0));
                emit(c, r)?;
                c.patch_jump(jmp_short, c.ops.len());
            }
            BinaryOp::Nullish => {
                emit(c, l)?;
                emit(c, r)?;
                c.push(OpCode::NullCoalesce);
            }
            _ => {
                emit(c, l)?;
                emit(c, r)?;
                let opc = match op {
                    BinaryOp::Add => OpCode::Add,
                    BinaryOp::Sub => OpCode::Sub,
                    BinaryOp::Mul => OpCode::Mul,
                    BinaryOp::Div => OpCode::Div,
                    BinaryOp::Mod => OpCode::Mod,
                    BinaryOp::Eq => OpCode::Eq,
                    BinaryOp::NotEq => OpCode::NotEq,
                    BinaryOp::EqStrict => OpCode::EqStrict,
                    BinaryOp::NotEqStrict => OpCode::NotEqStrict,
                    BinaryOp::Lt => OpCode::Lt,
                    BinaryOp::Gt => OpCode::Gt,
                    BinaryOp::LtEq => OpCode::LtEq,
                    BinaryOp::GtEq => OpCode::GtEq,
                    _ => unreachable!(),
                };
                c.push(opc);
            }
        },

        ExprKind::Ternary(cond, th, el) => {
            emit(c, cond)?;
            let jmp_false = c.push(OpCode::JumpIfFalse(0));
            emit(c, th)?;
            let jmp_end = c.push(OpCode::Jump(0));
            let else_start = c.ops.len();
            c.patch_jump(jmp_false, else_start);
            emit(c, el)?;
            c.patch_jump(jmp_end, c.ops.len());
        }

        ExprKind::Array(items) => {
            for it in items {
                emit(c, it)?;
            }
            c.push(OpCode::MakeArray(items.len() as u32));
        }

        ExprKind::Object(entries) => {
            for (k, v) in entries {
                let ki = c.intern_string(k);
                c.push(OpCode::PushObjectKey(ki));
                emit(c, v)?;
            }
            c.push(OpCode::MakeObject(entries.len() as u32));
        }

        ExprKind::Template(parts) => {
            let empty = c.intern_string("");
            c.push(OpCode::PushString(empty));
            for part in parts {
                match part {
                    TemplatePart::Text(t) => {
                        let i = c.intern_string(t);
                        c.push(OpCode::PushString(i));
                        c.push(OpCode::TemplateAppend);
                    }
                    TemplatePart::Expr(e) => {
                        emit(c, e)?;
                        c.push(OpCode::TemplateAppend);
                    }
                }
            }
        }

        ExprKind::Call(callee, args) => {
            let name = match &callee.kind {
                ExprKind::Identifier(n) => n.clone(),
                _ => {
                    return Err(Diagnostic::parse(
                        "only named function calls are supported (e.g. `len(x)`)",
                        e.span,
                    ))
                }
            };
            for a in args {
                emit(c, a)?;
            }
            let ni = c.intern_string(&name);
            c.push(OpCode::CallBuiltin(ni, args.len() as u32));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse;
    use super::*;

    fn compile_src(s: &str) -> Chunk {
        compile(&parse(s).unwrap()).unwrap()
    }

    #[test]
    fn compile_add() {
        let c = compile_src("1 + 2");
        assert_eq!(c.ops.len(), 4);
        assert!(matches!(c.ops[0], OpCode::PushNum(n) if n == 1.0));
        assert!(matches!(c.ops[1], OpCode::PushNum(n) if n == 2.0));
        assert!(matches!(c.ops[2], OpCode::Add));
        assert!(matches!(c.ops[3], OpCode::Return));
    }

    #[test]
    fn compile_scope_ref() {
        let c = compile_src("$state.count");
        let ops: Vec<_> = c
            .ops
            .iter()
            .filter(|o| !matches!(o, OpCode::Return))
            .collect();
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], OpCode::PushScopeRef(_)));
        assert!(matches!(ops[1], OpCode::MemberGet(_)));
    }

    #[test]
    fn compile_template() {
        let c = compile_src("`count=${42}`");
        let pushes = c
            .ops
            .iter()
            .filter(|o| matches!(o, OpCode::PushString(_)))
            .count();
        let appends = c
            .ops
            .iter()
            .filter(|o| matches!(o, OpCode::TemplateAppend))
            .count();
        assert!(pushes >= 2);
        assert_eq!(appends, 2);
    }
}
