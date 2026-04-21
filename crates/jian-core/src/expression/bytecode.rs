//! Stack-machine bytecode. Compact `Vec<OpCode>` with a const pool for strings,
//! scope paths, and object-key lists.

#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    // Constants
    PushNum(f64),
    PushBool(bool),
    PushNull,
    PushString(u32),    // index into string pool
    PushScopeRef(u32),  // index into scope-path pool, loads from StateGraph
    MakeArray(u32),     // pop N from stack, push array
    MakeObject(u32),    // pop N key-value pairs, push object
    PushObjectKey(u32), // index into string pool (key marker)

    // Access
    MemberGet(u32), // pop obj, push obj.<string>
    IndexGet,       // pop index, pop obj, push

    // Unary
    Not,
    Negate,
    UnaryPlus,

    // Binary
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    EqStrict,
    NotEqStrict,
    Lt,
    Gt,
    LtEq,
    GtEq,

    // Short-circuit uses jumps
    JumpIfFalse(i32), // relative offset to skip then-branch
    JumpIfTrue(i32),
    Jump(i32),

    // Null coalesce
    NullCoalesce, // pop two, push left if not null else right

    // Template
    TemplateAppend, // pop two strings-or-coerce, push concatenated

    // Call
    CallBuiltin(u32, u32), // (name_index, argc)

    // Result
    Return,
}

#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub ops: Vec<OpCode>,
    pub strings: Vec<String>,
    pub scope_paths: Vec<String>,
}

impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(i) = self.strings.iter().position(|x| x == s) {
            return i as u32;
        }
        self.strings.push(s.to_owned());
        (self.strings.len() - 1) as u32
    }

    pub fn intern_scope_path(&mut self, p: &str) -> u32 {
        if let Some(i) = self.scope_paths.iter().position(|x| x == p) {
            return i as u32;
        }
        self.scope_paths.push(p.to_owned());
        (self.scope_paths.len() - 1) as u32
    }

    pub fn push(&mut self, op: OpCode) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    pub fn patch_jump(&mut self, at: usize, target: usize) {
        let offset = target as i32 - at as i32 - 1;
        match &mut self.ops[at] {
            OpCode::JumpIfFalse(o) | OpCode::JumpIfTrue(o) | OpCode::Jump(o) => *o = offset,
            _ => panic!("patch_jump on non-jump"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_dedupes() {
        let mut c = Chunk::new();
        let a = c.intern_string("hi");
        let b = c.intern_string("hi");
        let cc = c.intern_string("bye");
        assert_eq!(a, b);
        assert_ne!(a, cc);
        assert_eq!(c.strings.len(), 2);
    }

    #[test]
    fn patch_jump_computes_offset() {
        let mut c = Chunk::new();
        c.push(OpCode::PushNum(1.0));
        let jmp = c.push(OpCode::JumpIfFalse(0));
        c.push(OpCode::PushNum(2.0));
        let here = c.ops.len();
        c.patch_jump(jmp, here);
        match c.ops[jmp] {
            OpCode::JumpIfFalse(o) => assert_eq!(o, 1),
            _ => panic!(),
        }
    }
}
