struct LowercaseMapping {
    source: u32,
    lowercase: u32,
}

fn mappings() -> &'static [LowercaseMapping] {
    include!("lowercase_table.rs")
}

// #NOTE: Backreferences use the established Unicode 15.0 simple-lowercase table, not literal-regex equivalence sets.
#[unsafe(no_mangle)]
extern "C" fn ers_regex_lowercase(character: u32, ascii_only: u32) -> u32 {
    if character < 128 {
        (character as u8).to_ascii_lowercase() as u32
    } else if ascii_only != 0 {
        character
    } else {
        let mappings = mappings();
        mappings
            .binary_search_by_key(&character, |mapping| mapping.source)
            .map_or(character, |index| mappings[index].lowercase)
    }
}
