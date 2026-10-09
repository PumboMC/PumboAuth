//! AuthMe import (CSV with `nick,hash` rows).

use crate::hashing::{HashFormat, detect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub name: String,
    pub hash: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportResult {
    pub rows: Vec<ImportRow>,
    pub invalid: u32,
}

/// Parses an export like `sqlite3 -csv authme.db "SELECT realname, password FROM authme"`.
/// Accepts `,`, `;` or tab separators, optional quotes and an optional header row.
pub fn parse_authme_csv(text: &str) -> ImportResult {
    let mut out = ImportResult::default();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let sep = [',', ';', '\t'].into_iter().find(|s| line.contains(*s));
        let Some(sep) = sep else {
            out.invalid += 1;
            continue;
        };
        let mut cols = line.split(sep).map(|c| c.trim().trim_matches('"').trim());
        let (Some(name), Some(hash)) = (cols.next(), cols.next()) else {
            out.invalid += 1;
            continue;
        };
        if i == 0 && detect(hash) == HashFormat::Unknown && !name.is_empty() {
            // header row such as "realname,password"
            continue;
        }
        if name.is_empty() || detect(hash) == HashFormat::Unknown {
            out.invalid += 1;
            continue;
        }
        out.rows.push(ImportRow { name: name.to_string(), hash: hash.to_string() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exports() {
        let csv = "realname,password\n\"Steve\",\"$SHA$abc$7eea1a3165d1bf71483f6a0fe371becbdfbab9e6d70ca0480b4ff5baadbcf881\"\nAlex;$2y$05$oac08OAD6QQQAftaNoKgLeRMW9f5V/AMQ60X77jtjS9NvH5k9qgIa\n\nbroken line\nBob,plain\n";
        let r = parse_authme_csv(csv);
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0].name, "Steve");
        assert!(r.rows[1].hash.starts_with("$2y$"));
        assert_eq!(r.invalid, 2);
    }
}
