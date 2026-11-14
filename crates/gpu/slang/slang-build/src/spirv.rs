use std::collections::HashSet;

// SPIR-V core opcodes and binary header layout.
const HEADER_WORDS: usize = 5;
const MAGIC: u32 = 0x0723_0203;
const TYPE_VOID: u16 = 19;
const TYPE_BOOL: u16 = 20;
const TYPE_INT: u16 = 21;
const TYPE_FLOAT: u16 = 22;
const TYPE_VECTOR: u16 = 23;
const TYPE_MATRIX: u16 = 24;
const TYPE_IMAGE: u16 = 25;
const TYPE_SAMPLER: u16 = 26;
const TYPE_SAMPLED_IMAGE: u16 = 27;
const TYPE_ARRAY: u16 = 28;
const TYPE_RUNTIME_ARRAY: u16 = 29;
const TYPE_STRUCT: u16 = 30;
const TYPE_OPAQUE: u16 = 31;
const TYPE_POINTER: u16 = 32;
const TYPE_FUNCTION: u16 = 33;
const TYPE_PIPE: u16 = 38;
const TYPE_FORWARD_POINTER: u16 = 39;
const UNDEF: u16 = 1;
const CONSTANT_TRUE: u16 = 41;
const CONSTANT_FALSE: u16 = 42;
const CONSTANT: u16 = 43;
const CONSTANT_COMPOSITE: u16 = 44;
const CONSTANT_SAMPLER: u16 = 45;
const CONSTANT_NULL: u16 = 46;
const SPEC_CONSTANT_TRUE: u16 = 48;
const SPEC_CONSTANT_FALSE: u16 = 49;
const SPEC_CONSTANT: u16 = 50;
const SPEC_CONSTANT_COMPOSITE: u16 = 51;
const SPEC_CONSTANT_OP: u16 = 52;
const FUNCTION: u16 = 54;
const VARIABLE: u16 = 59;
const VECTOR_SHUFFLE: u32 = 79;
const COMPOSITE_EXTRACT: u32 = 81;
const COMPOSITE_INSERT: u32 = 82;

struct Declaration<'a> {
    words: &'a [u32],
    result: u32,
    dependencies: Vec<u32>,
}

/// Some mobile Vulkan compilers cannot resolve forward storage-pointer types.
/// Emit the same nonrecursive declarations in dependency order, with their IDs,
/// layouts, constants, annotations and executable instructions unchanged.
pub fn order_declarations(code: &[u8]) -> Vec<u8> {
    assert!(
        code.len().is_multiple_of(size_of::<u32>()),
        "unaligned SPIR-V code"
    );
    let words = code
        .chunks_exact(size_of::<u32>())
        .map(|word| u32::from_le_bytes(word.try_into().expect("SPIR-V word")))
        .collect::<Vec<_>>();
    assert!(
        words.len() >= HEADER_WORDS && words[0] == MAGIC,
        "invalid SPIR-V header"
    );
    let mut instructions = Vec::new();
    let mut offset = HEADER_WORDS;
    while offset < words.len() {
        let count = (words[offset] >> 16) as usize;
        assert!(
            count != 0 && count <= words.len() - offset,
            "invalid SPIR-V instruction length"
        );
        instructions.push(&words[offset..offset + count]);
        offset += count;
    }
    if !instructions
        .iter()
        .any(|words| words[0] as u16 == TYPE_FORWARD_POINTER)
    {
        return code.to_vec();
    }
    let start = instructions
        .iter()
        .position(|words| (TYPE_VOID..=TYPE_FORWARD_POINTER).contains(&(words[0] as u16)))
        .expect("SPIR-V type declarations");
    let end = instructions
        .iter()
        .position(|words| words[0] as u16 == FUNCTION)
        .expect("SPIR-V function definitions");
    let mut declarations = Vec::new();
    for &words in &instructions[start..end] {
        let opcode = words[0] as u16;
        if opcode == TYPE_FORWARD_POINTER {
            continue;
        }
        let (result, dependencies) = if (TYPE_VOID..=TYPE_PIPE).contains(&opcode) {
            let dependencies = match opcode {
                TYPE_VECTOR | TYPE_MATRIX | TYPE_IMAGE | TYPE_SAMPLED_IMAGE
                | TYPE_RUNTIME_ARRAY => vec![words[2]],
                TYPE_ARRAY => words[2..4].to_vec(),
                TYPE_STRUCT | TYPE_FUNCTION => words[2..].to_vec(),
                TYPE_POINTER => vec![words[3]],
                TYPE_VOID
                | TYPE_BOOL
                | TYPE_INT
                | TYPE_FLOAT
                | TYPE_SAMPLER
                | TYPE_OPAQUE
                | 34..=TYPE_PIPE => Vec::new(),
                _ => unreachable!("SPIR-V core type opcode"),
            };
            (words[1], dependencies)
        } else {
            let mut dependencies = vec![words[1]];
            match opcode {
                CONSTANT_COMPOSITE | SPEC_CONSTANT_COMPOSITE => {
                    dependencies.extend_from_slice(&words[3..])
                }
                SPEC_CONSTANT_OP => {
                    let operands = &words[4..];
                    dependencies.extend_from_slice(match words[3] {
                        VECTOR_SHUFFLE | COMPOSITE_INSERT => &operands[..2],
                        COMPOSITE_EXTRACT => &operands[..1],
                        _ => operands,
                    });
                }
                VARIABLE => dependencies.extend_from_slice(words.get(4..).unwrap_or_default()),
                UNDEF | CONSTANT_TRUE | CONSTANT_FALSE | CONSTANT | CONSTANT_SAMPLER
                | CONSTANT_NULL | SPEC_CONSTANT_TRUE | SPEC_CONSTANT_FALSE | SPEC_CONSTANT => {}
                _ => panic!("unsupported SPIR-V global declaration opcode {opcode}"),
            }
            (words[2], dependencies)
        };
        declarations.push(Declaration {
            words,
            result,
            dependencies,
        });
    }
    let definitions = declarations
        .iter()
        .map(|declaration| declaration.result)
        .collect::<HashSet<_>>();
    let mut ready = HashSet::new();
    let mut ordered = Vec::new();
    while !declarations.is_empty() {
        let before = declarations.len();
        declarations.retain(|declaration| {
            if declaration
                .dependencies
                .iter()
                .all(|id| !definitions.contains(id) || ready.contains(id))
            {
                ordered.push(declaration.words);
                ready.insert(declaration.result);
                false
            } else {
                true
            }
        });
        assert_ne!(
            before,
            declarations.len(),
            "recursive portable SPIR-V declaration graph requires forward pointers"
        );
    }
    words[..HEADER_WORDS]
        .iter()
        .copied()
        .chain(
            instructions[..start]
                .iter()
                .flat_map(|words| words.iter().copied()),
        )
        .chain(ordered.iter().flat_map(|words| words.iter().copied()))
        .chain(
            instructions[end..]
                .iter()
                .flat_map(|words| words.iter().copied()),
        )
        .flat_map(u32::to_le_bytes)
        .collect()
}
