use crate::{
    cli::{Cli, Sort, Units},
    scan::{Entry, Kind, Tree},
    ui::safe_text,
};
use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    io::{self, Write},
    path::PathBuf,
};
pub fn size(bytes: u64, units: Units) -> String {
    match units {
        Units::Bytes => bytes.to_string(),
        Units::Fixed(div, label) => format!("{:.1} {label}", bytes as f64 / div as f64),
        Units::Human | Units::Si => {
            let base = if matches!(units, Units::Si) {
                1000.0
            } else {
                1024.0
            };
            let labels = if matches!(units, Units::Si) {
                ["B", "kB", "MB", "GB", "TB", "PB", "EB"]
            } else {
                ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]
            };
            let mut n = bytes as f64;
            let mut i = 0;
            while n >= base && i < 6 {
                n /= base;
                i += 1;
            }
            format!("{n:.1} {}", labels[i])
        }
    }
}
pub fn counts(tree: &Tree) -> Vec<u64> {
    let mut counts: Vec<u64> = tree.dirs.iter().map(|d| d.entries.len() as u64).collect();
    for id in (1..counts.len()).rev() {
        if let Some(p) = tree.dirs[id].parent {
            counts[p] = counts[p].saturating_add(counts[id]);
        }
    }
    counts
}
fn metric(e: &Entry, c: &Cli) -> u64 {
    if c.apparent { e.apparent } else { e.allocated }
}
fn compare(a: &Entry, b: &Entry, c: &Cli, counts: &[u64]) -> Ordering {
    let order = match c.sort {
        Sort::Size => metric(b, c).cmp(&metric(a, c)),
        Sort::Name => a.name.cmp(&b.name),
        Sort::Count => b
            .child
            .map_or(0, |i| counts[i])
            .cmp(&a.child.map_or(0, |i| counts[i])),
    }
    .then_with(|| a.name.cmp(&b.name));
    if c.reverse { order.reverse() } else { order }
}
pub fn write(tree: &Tree, c: &Cli, out: &mut impl Write) -> io::Result<()> {
    let root = &tree.dirs[0];
    let total = if c.apparent {
        root.apparent
    } else {
        root.allocated
    };
    if c.summary {
        let v = serde_json::json!({"allocated":root.allocated,"apparent":root.apparent,"files":tree.files,"directories":tree.dirs.len(),"errors":tree.errors,"elapsed_seconds":tree.elapsed.as_secs_f64()});
        writeln!(out, "{v}")?;
        return Ok(());
    }
    if c.quiet {
        return Ok(());
    }
    if c.summarize {
        return writeln!(
            out,
            "{}  {}",
            size(total, c.units),
            safe_text(tree.root.as_os_str())
        );
    }
    let counts = counts(tree);
    if let Some(top) = c.top {
        // Keep only the largest N file rows; do not duplicate every name/path.
        let mut heap = BinaryHeap::new();
        let limit = top.min(tree.files as usize);
        for (dir, d) in tree.dirs.iter().enumerate() {
            for (entry, e) in d.entries.iter().enumerate() {
                if e.kind == Kind::File && limit > 0 {
                    let row = Reverse((metric(e, c), dir, entry));
                    if heap.len() < limit {
                        heap.push(row)
                    } else if heap.peek().is_some_and(|v| row < *v) {
                        heap.pop();
                        heap.push(row);
                    }
                }
            }
        }
        let mut rows: Vec<_> = heap.into_iter().map(|Reverse(v)| v).collect();
        rows.sort_unstable_by_key(|&(bytes, dir, entry)| (Reverse(bytes), dir, entry));
        if c.reverse {
            rows.reverse();
        }
        for (bytes, dir, entry) in rows {
            let mut components = Vec::new();
            let mut id = dir;
            while let Some(parent) = tree.dirs[id].parent {
                components.push(tree.dirs[id].name.clone());
                id = parent;
            }
            let mut path = PathBuf::new();
            for name in components.into_iter().rev() {
                path.push(name)
            }
            path.push(&tree.dirs[dir].entries[entry].name);
            writeln!(
                out,
                "{:>14}  {}",
                size(bytes, c.units),
                safe_text(path.as_os_str())
            )?;
        }
    } else if c.depth > 0 {
        let mut stack = vec![(0usize, PathBuf::new(), 1usize)];
        while let Some((dir, prefix, depth)) = stack.pop() {
            let mut entries: Vec<_> = tree.dirs[dir].entries.iter().collect();
            entries.sort_unstable_by(|a, b| compare(a, b, c, &counts));
            let mut children = Vec::new();
            for e in entries {
                let path = prefix.join(&e.name);
                if c.show_count {
                    write!(out, "{:>9} ", e.child.map_or(0, |id| counts[id]))?;
                }
                writeln!(
                    out,
                    "{:>14}  {}{}",
                    size(metric(e, c), c.units),
                    safe_text(path.as_os_str()),
                    if e.kind == Kind::Directory { "/" } else { "" }
                )?;
                if depth < c.depth
                    && let Some(child) = e.child
                {
                    children.push((child, path, depth + 1));
                }
            }
            stack.extend(children.into_iter().rev());
        }
    }
    writeln!(
        out,
        "{:>14}  TOTAL ({} entries, {} directories, {} errors; {:.3}s)",
        size(total, c.units),
        tree.files,
        tree.dirs.len(),
        tree.errors,
        tree.elapsed.as_secs_f64()
    )
}
