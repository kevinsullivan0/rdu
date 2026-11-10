//! Scan-scope filters. Directory exclusions run before admission/descent.
use crate::scan::Kind;
use regex_lite::Regex;
use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};
#[derive(Debug, Default)]
pub struct Filters {
    pub no_hidden: bool,
    exact: Vec<PathBuf>,
    patterns: Vec<Regex>,
    include: Vec<String>,
    exclude: Vec<String>,
}
fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}
impl Filters {
    pub fn new(
        root: &Path,
        no_hidden: bool,
        ignore: &[PathBuf],
        patterns: &[String],
        include: Vec<String>,
        exclude: Vec<String>,
    ) -> Result<Self, String> {
        if patterns.len() > 256 || patterns.iter().map(String::len).sum::<usize>() > 65536 {
            return Err("directory regex patterns exceed limits".into());
        }
        let patterns = patterns
            .iter()
            .map(|s| Regex::new(s).map_err(|e| format!("invalid directory regex: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            no_hidden,
            exact: ignore
                .iter()
                .map(|p| {
                    normalize(&if p.is_absolute() {
                        p.clone()
                    } else {
                        root.join(p)
                    })
                })
                .collect(),
            patterns,
            include,
            exclude,
        })
    }
    pub fn accepts(&self, root: &Path, parent: &Path, name: &OsStr, kind: Kind) -> bool {
        if kind == Kind::Directory {
            if self.no_hidden && name.as_bytes().starts_with(b".") {
                return false;
            }
            if self.exact.is_empty() && self.patterns.is_empty() {
                return true;
            }
            let path = root.join(parent).join(name);
            return !(!self.exact.is_empty() && self.exact.contains(&normalize(&path))
                || self
                    .patterns
                    .iter()
                    .any(|r| r.is_match(&path.to_string_lossy())));
        }
        if self.include.is_empty() && self.exclude.is_empty() {
            return true;
        }
        let extension = Path::new(name)
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        (self.include.is_empty() || self.include.contains(&extension))
            && !self.exclude.contains(&extension)
    }
}
