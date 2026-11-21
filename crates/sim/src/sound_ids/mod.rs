//! Sound ids: from the numbers and names game code uses to the sounds that play.
//!
//! Game code asks for music and sound effects by a number (left over from the original console version) or
//! by name. Seven tables map them to a sound: one for music and six for sound effects. Lookups remember the
//! last sound found, and a name lookup that matches the remembered one answers from it without searching,
//! whichever table it came from (as in the original).

mod table;

/// One table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundDef {
    /// The name and number the entry is found by.
    pub key_name: &'static str,
    pub key_id: i32,
    /// The sound's own name and number.
    pub name: &'static str,
    /// The sound file (clip) to play.
    pub file: &'static str,
    pub id: i32,
    /// Music that loops.
    pub looping: bool,
}

/// What a lookup is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Music,
    SoundEffect,
}

/// The seven tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    Music,
    System,
    Field,
    Event,
    Skill,
    Demon,
    FieldEffect,
}

impl Table {
    pub fn entries(self) -> &'static [SoundDef] {
        match self {
            Table::Music => table::BGM,
            Table::System => table::SYS,
            Table::Field => table::FLD,
            Table::Event => table::EVT,
            Table::Skill => table::SKIL,
            Table::Demon => table::MSE,
            Table::FieldEffect => table::FESE,
        }
    }

    /// The table a name belongs to.
    pub fn for_name(kind: Kind, name: &str) -> Table {
        if kind == Kind::Music {
            return Table::Music;
        }
        if name.starts_with('D') {
            Table::Demon
        } else if name.starts_with('E') {
            Table::Event
        } else if name.starts_with("FESE") {
            Table::FieldEffect
        } else if name.starts_with('F') {
            Table::Field
        } else if name.starts_with("SKIL") {
            Table::Skill
        } else {
            Table::System
        }
    }

    /// The table a number is looked up in.
    pub fn for_number(kind: Kind, n: i32) -> Table {
        if kind == Kind::Music {
            return Table::Music;
        }
        let u = n as u32;
        let within = |lo: u32, len: u32| u.wrapping_sub(lo) < len;
        if within(0x40_0000, 0x40_0000) {
            Table::Field
        } else if within(0x100_0000, 0xE6_0000) {
            Table::Event
        } else if within(0x1E6_0000, 0x1A_0000) {
            Table::System
        } else if within(0x200_0000, 0x200_0000) {
            Table::Skill
        } else if within(0x500_0000, 0x200_0000) {
            Table::Demon
        } else if within(0x900_0000, 0x100_0000) {
            Table::FieldEffect
        } else if within(0x1000_0000, 0x2000_0000) {
            Table::Demon
        } else {
            Table::System
        }
    }
}

/// The remembered result of the last lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Found {
    name: Option<&'static str>,
    key_name: Option<&'static str>,
    id: i32,
}

/// Lookup state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundIds {
    found: Found,
    found_def: Option<SoundDef>,
}

impl Default for SoundIds {
    fn default() -> Self {
        Self { found: Found { name: None, key_name: None, id: -1 }, found_def: None }
    }
}

impl SoundIds {
    pub fn new() -> Self {
        Self::default()
    }

    fn remember(&mut self, d: &SoundDef) {
        self.found = Found { name: Some(d.name), key_name: Some(d.key_name), id: d.id };
        self.found_def = Some(*d);
    }

    /// The sound id for a number, or -1. The first entry with that number wins.
    pub fn find_number(&mut self, kind: Kind, n: i32) -> i32 {
        match Table::for_number(kind, n).entries().iter().find(|d| d.key_id == n) {
            Some(d) => {
                self.remember(d);
                d.id
            }
            None => {
                self.found.name = Some("");
                -1
            }
        }
    }

    /// The sound id for a name, or -1.
    pub fn find_name(&mut self, kind: Kind, name: &str) -> i32 {
        if self.found.key_name == Some(name) {
            return self.found.id;
        }
        match Table::for_name(kind, name).entries().iter().find(|d| d.key_name == name) {
            Some(d) => {
                self.remember(d);
                d.id
            }
            None => {
                self.found.name = Some("");
                -1
            }
        }
    }

    /// The entry of the last successful lookup.
    pub fn last(&self) -> Option<&SoundDef> {
        self.found_def.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_sizes() {
        let n: Vec<usize> = [
            Table::Music,
            Table::System,
            Table::Field,
            Table::Event,
            Table::Skill,
            Table::Demon,
            Table::FieldEffect,
        ]
        .iter()
        .map(|t| t.entries().len())
        .collect();
        assert_eq!(n, [327, 88, 584, 469, 256, 1633, 69]);
    }

    #[test]
    fn title_sounds() {
        let mut s = SoundIds::new();
        assert_eq!(s.find_number(Kind::SoundEffect, 0x3A_0001), 0x3A_0001);
        assert_eq!(s.last().unwrap().file, "BGM_CONF_SE00");
        assert_eq!(s.find_number(Kind::SoundEffect, 10), 10);
        assert_eq!(s.last().unwrap().file, "COMSE_SE0A");
        let id = s.find_name(Kind::Music, "BGM_CONF_BGM00");
        assert_eq!(id, 0x3A_0000);
        let d = s.last().unwrap();
        assert!(d.looping);
        assert_eq!(d.file, "BGM_CONF_BGM00_L_");
    }

    #[test]
    fn entries_are_found_by_name_unless_stored_in_another_table() {
        // Copies live in the table of the entry they copy, so a copy's own name may point to another table,
        // where the original does not find it either.
        let mut s = SoundIds::new();
        for t in [Table::System, Table::Field, Table::Event, Table::Skill, Table::Demon, Table::FieldEffect] {
            for d in t.entries() {
                let got = s.find_name(Kind::SoundEffect, d.key_name);
                if Table::for_name(Kind::SoundEffect, d.key_name) == t {
                    assert_eq!(got, d.id, "{}", d.key_name);
                } else {
                    assert_ne!(d.key_name, d.name);
                }
            }
        }
    }

    #[test]
    fn misses_and_the_remembered_entry() {
        let mut s = SoundIds::new();
        assert_eq!(s.find_number(Kind::Music, 0x7FFF_FFFF), -1);
        assert_eq!(s.find_number(Kind::SoundEffect, 10), 10);
        // A miss keeps the remembered key, so its name still answers from memory.
        assert_eq!(s.find_number(Kind::SoundEffect, 0x7FFF_FFFF), -1);
        assert_eq!(s.find_name(Kind::Music, "COMSE_SE0A"), 10);
    }
}
