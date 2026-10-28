//! Comment-preserving edits to config.toml.
//!
//! The settings window never re-serialises the configuration from structs:
//! it edits the parsed `toml_edit` document in place, so comments, blank
//! lines, key order and formatting the user wrote survive a save.

use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// Replace `slot` with `v`, keeping the comments/spacing around the old value.
fn put(slot: Option<&Item>, mut v: Value) -> Item {
    if let Some(old) = slot.and_then(Item::as_value) {
        *v.decor_mut() = old.decor().clone();
    }
    Item::Value(v)
}

/// Set (`Some`) or remove (`None`) the value at `path` below `t`. Missing
/// intermediate tables are created: as inline tables when `inline` (e.g. a
/// rule's `match = { ... }`), otherwise as `[section]` tables.
pub fn set_in(t: &mut dyn TableLike, path: &[&str], v: Option<Value>, inline: bool) -> Result<(), String> {
    let (first, rest) = path.split_first().ok_or("empty path")?;
    if rest.is_empty() {
        match v {
            Some(v) => {
                let item = put(t.get(first), v);
                t.insert(first, item);
            }
            None => {
                t.remove(first);
            }
        }
        return Ok(());
    }
    if t.get(first).is_none() {
        if v.is_none() {
            return Ok(());
        }
        let child = if inline {
            Item::Value(Value::InlineTable(InlineTable::new()))
        } else {
            let mut tb = Table::new();
            tb.set_implicit(true);
            Item::Table(tb)
        };
        t.insert(first, child);
    }
    let child = t.get_mut(first).and_then(Item::as_table_like_mut).ok_or_else(|| format!("`{first}` is not a table"))?;
    set_in(child, rest, v, inline)?;
    // Inline tables cannot hold comments, so normalising their spacing loses
    // nothing; drop one that became empty (e.g. last match field removed).
    if let Some(it) = t.get_mut(first).and_then(Item::as_inline_table_mut) {
        if it.is_empty() {
            t.remove(first);
        } else {
            it.fmt();
        }
    }
    Ok(())
}

pub fn set(doc: &mut DocumentMut, path: &[&str], v: impl Into<Value>) -> Result<(), String> {
    set_in(doc.as_table_mut(), path, Some(v.into()), false)
}

pub fn remove(doc: &mut DocumentMut, path: &[&str]) -> Result<(), String> {
    set_in(doc.as_table_mut(), path, None, false)
}

/// Print `doc` with the line endings of the file it came from (toml_edit
/// always prints `\n`; Notepad and Windows checkouts often use `\r\n`).
pub fn render(doc: &DocumentMut, crlf: bool) -> String {
    let s = doc.to_string();
    if crlf {
        s.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        s
    }
}

pub fn uses_crlf(text: &str) -> bool {
    text.contains("\r\n")
}

pub fn get<'a>(doc: &'a DocumentMut, path: &[&str]) -> Option<&'a Item> {
    let mut item = doc.as_item();
    for k in path {
        item = item.as_table_like()?.get(k)?;
    }
    Some(item)
}

/// `[binds]` as (chord, action) rows in file order. Non-string values are
/// shown as their TOML text so they can be fixed rather than silently lost.
pub fn binds(doc: &DocumentMut) -> Vec<(String, String)> {
    let Some(t) = doc.get("binds").and_then(Item::as_table_like) else { return Vec::new() };
    t.iter()
        .map(|(k, v)| (k.to_string(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string().trim().to_string())))
        .collect()
}

/// Rewrite `[binds]` from rows, keeping the comments attached to chords
/// that still exist. Rows with an empty chord are skipped.
pub fn set_binds(doc: &mut DocumentMut, rows: &[(String, String)]) {
    if doc.get("binds").and_then(Item::as_table).is_none() {
        doc.insert("binds", Item::Table(Table::new()));
    }
    let t = doc["binds"].as_table_mut().expect("binds is a table");
    let old = t.clone();
    t.clear();
    for (chord, action) in rows {
        let chord = chord.trim();
        if chord.is_empty() {
            continue;
        }
        match old.get_key_value(chord) {
            Some((k, item)) => {
                let item = put(Some(item), Value::from(action.as_str()));
                t.insert_formatted(k, item);
            }
            None => {
                t.insert(chord, toml_edit::value(action.as_str()));
            }
        }
    }
}

pub fn rules(doc: &DocumentMut) -> Option<&ArrayOfTables> {
    doc.get("rule").and_then(Item::as_array_of_tables)
}

pub fn rule_count(doc: &DocumentMut) -> usize {
    rules(doc).map_or(0, |a| a.len())
}

pub fn rule_mut(doc: &mut DocumentMut, i: usize) -> Option<&mut Table> {
    doc.get_mut("rule").and_then(Item::as_array_of_tables_mut).and_then(|a| a.get_mut(i))
}

/// Set or clear a field of rule `i`; `["match", "process"]` edits the
/// inline `match` table.
pub fn set_rule(doc: &mut DocumentMut, i: usize, path: &[&str], v: Option<Value>) -> Result<(), String> {
    let t = rule_mut(doc, i).ok_or("no such rule")?;
    set_in(t, path, v, true)
}

pub fn add_rule(doc: &mut DocumentMut, rule: Table) {
    if doc.get("rule").and_then(Item::as_array_of_tables).is_none() {
        doc.insert("rule", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    doc["rule"].as_array_of_tables_mut().expect("rule array").push(rule);
}

pub fn remove_rule(doc: &mut DocumentMut, i: usize) {
    if let Some(a) = doc.get_mut("rule").and_then(Item::as_array_of_tables_mut) {
        if i < a.len() {
            a.remove(i);
        }
        if a.is_empty() {
            doc.remove("rule");
        }
    }
}

/// Swap rules `i` and `i + 1` (rule order matters: later rules win).
/// Document positions stay with the slots so the file order changes too.
pub fn swap_rules(doc: &mut DocumentMut, i: usize) {
    let Some(a) = doc.get_mut("rule").and_then(Item::as_array_of_tables_mut) else { return };
    if i + 1 >= a.len() {
        return;
    }
    let mut tables: Vec<Table> = a.iter().cloned().collect();
    let (pi, pj) = (tables[i].position(), tables[i + 1].position());
    tables.swap(i, i + 1);
    tables[i].set_position(pi);
    tables[i + 1].set_position(pj);
    a.clear();
    for t in tables {
        a.push(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "# top comment\n[general]\ngaps_in = 5 # inner gap\n\n[binds]\n# launch\n\"$mod+return\" = \"exec wt.exe\"\n\"$mod+c\" = \"close\"\n\n[[rule]]\nname = \"a\"\nmatch = { process = \"a\\\\.exe\" }\nfloat = true\n\n[[rule]]\nname = \"b\"\nmatch = { class = \"B\" }\n";

    fn doc() -> DocumentMut {
        SRC.parse().unwrap()
    }

    #[test]
    fn set_keeps_comments() {
        let mut d = doc();
        set(&mut d, &["general", "gaps_in"], 9).unwrap();
        let s = d.to_string();
        assert!(s.contains("gaps_in = 9 # inner gap"), "{s}");
        assert!(s.starts_with("# top comment"));
    }

    #[test]
    fn set_creates_nested_sections() {
        let mut d = doc();
        set(&mut d, &["animations", "windows", "duration"], 250).unwrap();
        let s = d.to_string();
        assert!(s.contains("[animations.windows]\nduration = 250"), "{s}");
        assert!(!s.contains("[animations]\n"), "implicit parent header: {s}");
        assert_eq!(get(&d, &["animations", "windows", "duration"]).and_then(Item::as_integer), Some(250));
        remove(&mut d, &["animations", "windows", "duration"]).unwrap();
        assert!(get(&d, &["animations", "windows", "duration"]).is_none());
        // Removing something that does not exist is not an error.
        remove(&mut d, &["nope", "x"]).unwrap();
    }

    #[test]
    fn binds_roundtrip_keeps_order_and_comments() {
        let mut d = doc();
        let mut rows = binds(&d);
        assert_eq!(rows[0], ("$mod+return".into(), "exec wt.exe".into()));
        rows[0].1 = "exec cmd.exe".into();
        rows[1].0 = "$mod+q".into();
        rows.push(("alt+h".into(), "focus left".into()));
        set_binds(&mut d, &rows);
        let s = d.to_string();
        assert!(s.contains("# launch\n\"$mod+return\" = \"exec cmd.exe\""), "{s}");
        assert!(!s.contains("$mod+c"));
        let order: Vec<_> = binds(&d).into_iter().map(|r| r.0).collect();
        assert_eq!(order, ["$mod+return", "$mod+q", "alt+h"]);
        // The result still parses as the same TOML.
        s.parse::<DocumentMut>().unwrap();
    }

    #[test]
    fn rule_edits() {
        let mut d = doc();
        assert_eq!(rule_count(&d), 2);
        set_rule(&mut d, 1, &["match", "title"], Some("T".into())).unwrap();
        set_rule(&mut d, 1, &["workspace"], Some(3.into())).unwrap();
        set_rule(&mut d, 0, &["match", "process"], None).unwrap();
        let s = d.to_string();
        assert!(s.contains("match = { class = \"B\", title = \"T\" }"), "{s}");
        // Last match field removed -> the empty inline table goes too.
        assert!(!s.contains("match = {}") && !s.contains("match = { }"), "{s}");
        swap_rules(&mut d, 0);
        let s = d.to_string();
        assert!(s.find("name = \"b\"").unwrap() < s.find("name = \"a\"").unwrap(), "{s}");
        remove_rule(&mut d, 0);
        remove_rule(&mut d, 0);
        assert_eq!(rule_count(&d), 0);
        assert!(!d.to_string().contains("[[rule]]"));
        let mut t = Table::new();
        t.insert("name", toml_edit::value("new"));
        add_rule(&mut d, t);
        assert!(d.to_string().contains("[[rule]]\nname = \"new\""));
    }

    #[test]
    fn untouched_documents_print_identically() {
        let src = twm_core::config::DEFAULT_CONFIG;
        let lf = src.replace("\r\n", "\n");
        for src in [lf.clone(), lf.replace('\n', "\r\n")] {
            let d: DocumentMut = src.parse().unwrap();
            let out = render(&d, uses_crlf(&src));
            let first = src.lines().zip(out.lines()).position(|(a, b)| a != b);
            assert!(src == out, "first differing line: {first:?}");
        }
    }

    #[test]
    fn editing_the_default_config_keeps_every_comment() {
        let src = twm_core::config::DEFAULT_CONFIG;
        let mut d: DocumentMut = src.parse().unwrap();
        set(&mut d, &["general", "gaps_out"], 20).unwrap();
        set(&mut d, &["decoration", "active_border"], "rgb(ff0000)").unwrap();
        set(&mut d, &["animations", "windows", "duration"], 300).unwrap();
        let mut rows = binds(&d);
        rows.retain(|r| r.0 != "$mod+c");
        set_binds(&mut d, &rows);
        set_rule(&mut d, 0, &["workspace"], Some(2.into())).unwrap();
        let out = d.to_string();
        let comments = |t: &str| t.lines().filter(|l| l.trim_start().starts_with('#')).map(str::to_string).collect::<Vec<_>>();
        assert_eq!(comments(src), comments(&out));
        let r = twm_core::config::load_str(&out, "config.toml", None, &|_| None);
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        assert_eq!(r.config.general.gaps_out, 20);
        assert_eq!(r.config.animations.windows.duration, 300);
        assert_eq!(r.config.binds.len(), binds(&src.parse().unwrap()).len() - 1);
    }
}
