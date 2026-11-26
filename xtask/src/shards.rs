//! The gate's shards (`cargo xtask ci shard`, with `GATE_SHARD` of `GATE_SHARDS` set by the shared
//! gate workflow): which workspace crates each shard tests, and the checks that keep the split
//! complete.
//!
//! Each test shard builds and runs the tests of its own crates and what they depend on. The last
//! shard runs the `cargo xtask ci` steps that are not tests. [`GROUPS`] names the crates of the
//! first shards; every other workspace member goes to the [`Group::Rest`] shard, so a new crate is
//! tested without a change here. [`check`] fails when a named crate is not a workspace member or
//! is named twice, when a group is empty, and when a crate's features in its shard differ from
//! its features in `cargo test --workspace`. Cargo resolves features over the packages a command
//! selects, so a feature that only another crate's dev-dependencies turn on is off in a shard
//! without that crate, and the tests behind it do not build.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// What one shard runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// The tests of these workspace crates (package names).
    Crates(&'static [&'static str]),
    /// The tests of every workspace member no other group names.
    Rest,
    /// The `cargo xtask ci` steps that are not tests.
    Checks,
}

/// The shards in order; `test-shards` in `.github/workflows/ci.yml` is its length.
pub const GROUPS: &[Group] = &[
    Group::Crates(&["vectorcraft-ui-egui"]),
    Group::Crates(&["vectorcraft-engine"]),
    Group::Crates(&["vectorcraft", "vectorcraft-web", "vectorcraft-cli", "vectorcraft-mcp", "xtask"]),
    Group::Rest,
    Group::Checks,
];

/// A workspace member, as `cargo metadata` gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// The package id, which `-p` takes where the name would be ambiguous: craft-libs builds
    /// against this repo's geom and pathops by git, so the graph holds a second copy of each.
    pub id: String,
    pub version: String,
}

/// What one shard runs, with its crates resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shard {
    Tests(Vec<Member>),
    Checks,
}

/// The workspace members of `cargo metadata --no-deps` output, in its order.
pub fn members(meta: &Value) -> Result<Vec<Member>, String> {
    let ids: BTreeSet<&str> = meta["workspace_members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let members: Vec<Member> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["id"].as_str().is_some_and(|id| ids.contains(id)))
        .map(|p| {
            let field = |key: &str| p[key].as_str().map(str::to_owned).ok_or_else(|| format!("cargo metadata: a workspace package has no `{key}`"));
            Ok(Member { name: field("name")?, id: field("id")?, version: field("version")? })
        })
        .collect::<Result<_, String>>()?;
    if members.is_empty() {
        return Err("cargo metadata lists no workspace members".into());
    }
    Ok(members)
}

/// Each group of `groups` with its crates: a named crate must be a workspace member and named
/// once, there must be exactly one [`Group::Rest`], and no test shard may be empty.
pub fn assign(groups: &[Group], members: &[Member]) -> Result<Vec<Shard>, String> {
    const WHERE: &str = "xtask/src/shards.rs › GROUPS";
    let mut named: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, g) in groups.iter().enumerate() {
        if let Group::Crates(names) = g {
            if names.is_empty() {
                return Err(format!("shard {} names no crates ({WHERE})", i + 1));
            }
            for name in *names {
                if !members.iter().any(|m| m.name == *name) {
                    return Err(format!("shard {} names `{name}`, which is not a workspace member ({WHERE})", i + 1));
                }
                if let Some(first) = named.insert(name, i) {
                    return Err(format!("`{name}` is in shard {} and shard {} ({WHERE})", first + 1, i + 1));
                }
            }
        }
    }
    let rests = groups.iter().filter(|g| **g == Group::Rest).count();
    if rests != 1 {
        return Err(format!("{rests} shards take the remaining crates, not 1 ({WHERE})"));
    }
    let pick = |f: &dyn Fn(&Member) -> bool| members.iter().filter(|m| f(m)).cloned().collect::<Vec<_>>();
    groups
        .iter()
        .enumerate()
        .map(|(i, g)| match g {
            Group::Crates(names) => Ok(Shard::Tests(pick(&|m| names.contains(&m.name.as_str())))),
            Group::Rest => {
                let rest = pick(&|m| !named.contains_key(m.name.as_str()));
                if rest.is_empty() {
                    return Err(format!("shard {} takes the remaining crates, and there are none ({WHERE})", i + 1));
                }
                Ok(Shard::Tests(rest))
            }
            Group::Checks => Ok(Shard::Checks),
        })
        .collect()
}

/// The features `cargo tree -e normal,dev,build --prefix none -f '{p}|{f}'` shows for each of
/// `members`, by name; a crate the tree holds twice (once for the host, say) has two entries.
/// Other copies of a member's name (craft-libs' git copies of geom and pathops) are not members.
pub fn tree_features(tree: &str, members: &[Member]) -> BTreeMap<String, BTreeSet<String>> {
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in tree.lines() {
        let line = line.trim_end();
        let line = line.strip_suffix(" (*)").unwrap_or(line);
        let Some((package, features)) = line.split_once('|') else { continue };
        let Some(m) =
            members.iter().find(|m| package.strip_prefix(&format!("{} v{} (", m.name, m.version)).is_some_and(|source| !source.contains("://")))
        else {
            continue;
        };
        let mut features: Vec<&str> = features.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
        features.sort_unstable();
        found.entry(m.name.clone()).or_default().insert(features.join(","));
    }
    found
}

/// One line for each crate of `shard` whose features in `alone` (the shard's tree) differ from
/// `workspace` (the whole workspace's tree).
pub fn feature_differences(
    shard: usize,
    crates: &[Member],
    workspace: &BTreeMap<String, BTreeSet<String>>,
    alone: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let show = |set: Option<&BTreeSet<String>>| match set {
        None => "absent".to_owned(),
        Some(set) => set.iter().map(|f| format!("[{f}]")).collect::<Vec<_>>().join(" "),
    };
    crates
        .iter()
        .filter(|m| workspace.get(&m.name) != alone.get(&m.name))
        .map(|m| {
            format!("{}: features {} in shard {shard}, {} with the whole workspace", m.name, show(alone.get(&m.name)), show(workspace.get(&m.name)))
        })
        .collect()
}

/// `cargo tree` with the features of each package, for `select` (`--workspace` or `-p` ids).
fn tree(select: &[String]) -> Result<String, String> {
    let mut c = crate::cargo();
    // Without `--color never`, CARGO_TERM_COLOR=always (as the gate sets it) colors the ` (*)` marks.
    c.args(["tree", "--color", "never", "-e", "normal,dev,build", "--prefix", "none", "-f", "{p}|{f}"]).args(select);
    let out = c.output().map_err(|e| format!("cargo tree: failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo tree {}: exited with {}:\n{}", select.join(" "), out.status, String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `-p <id>` for each crate.
pub fn select(crates: &[Member]) -> Vec<String> {
    crates.iter().flat_map(|m| ["-p".to_owned(), m.id.clone()]).collect()
}

/// The package names of `crates`, for messages.
pub fn names(crates: &[Member]) -> String {
    crates.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
}

/// The shards of this workspace, after the checks in [`assign`] and [`feature_differences`].
pub fn check() -> Result<Vec<Shard>, String> {
    let members = members(&crate::metadata()?)?;
    let shards = assign(GROUPS, &members)?;
    let workspace = tree_features(&tree(&["--workspace".to_owned()])?, &members);
    let mut differences = Vec::new();
    for (i, shard) in shards.iter().enumerate() {
        if let Shard::Tests(crates) = shard {
            let alone = tree_features(&tree(&select(crates))?, &members);
            differences.extend(feature_differences(i + 1, crates, &workspace, &alone));
        }
    }
    if !differences.is_empty() {
        return Err(format!(
            "{} crate(s) build with other features in their shard than in `cargo test --workspace`, so some of their tests \
             would build differently or not at all (turn the feature on in the crate's own dev-dependencies, or move the crate \
             to the shard of the crate that turns it on):\n  {}",
            differences.len(),
            differences.join("\n  ")
        ));
    }
    Ok(shards)
}

/// `cargo xtask shards`: prints each shard's crates after [`check`], and returns the shards.
pub fn run() -> Result<Vec<Shard>, String> {
    let shards = check()?;
    for (i, shard) in shards.iter().enumerate() {
        match shard {
            Shard::Tests(crates) => println!("shard {}/{}: tests of {} crate(s): {}", i + 1, shards.len(), crates.len(), names(crates)),
            Shard::Checks => println!("shard {}/{}: the other ci steps", i + 1, shards.len()),
        }
    }
    println!("OK: every workspace member is tested on one shard, with the features `cargo test --workspace` gives it.");
    Ok(shards)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str) -> Member {
        Member { name: name.into(), id: format!("path+file:///w/{name}#0.1.0"), version: "0.1.0".into() }
    }

    fn crates(shard: &Shard) -> Vec<&str> {
        match shard {
            Shard::Tests(c) => c.iter().map(|m| m.name.as_str()).collect(),
            Shard::Checks => Vec::new(),
        }
    }

    #[test]
    fn the_rest_shard_takes_every_crate_no_group_names() {
        let ws = [m("a"), m("b"), m("c"), m("d")];
        let shards = assign(&[Group::Crates(&["c"]), Group::Rest, Group::Checks], &ws).unwrap();
        assert_eq!(shards.iter().map(crates).collect::<Vec<_>>(), [vec!["c"], vec!["a", "b", "d"], vec![]]);
        assert_eq!(shards[2], Shard::Checks);
    }

    #[test]
    fn a_crate_named_twice_or_unknown_fails() {
        let ws = [m("a"), m("b")];
        let twice = assign(&[Group::Crates(&["a"]), Group::Crates(&["a"]), Group::Rest], &ws).unwrap_err();
        assert!(twice.contains("`a` is in shard 1 and shard 2"), "{twice}");
        let unknown = assign(&[Group::Crates(&["z"]), Group::Rest], &ws).unwrap_err();
        assert!(unknown.contains("`z`, which is not a workspace member"), "{unknown}");
    }

    #[test]
    fn empty_groups_and_a_missing_or_second_rest_fail() {
        let ws = [m("a"), m("b")];
        assert!(assign(&[Group::Crates(&[]), Group::Rest], &ws).unwrap_err().contains("names no crates"));
        assert!(assign(&[Group::Crates(&["a", "b"]), Group::Rest], &ws).unwrap_err().contains("there are none"));
        assert!(assign(&[Group::Crates(&["a"])], &ws).unwrap_err().contains("0 shards take the remaining crates"));
        assert!(assign(&[Group::Rest, Group::Rest], &ws).unwrap_err().contains("2 shards take the remaining crates"));
    }

    #[test]
    fn every_workspace_member_is_tested_on_exactly_one_shard() {
        let ws = members(&crate::metadata().unwrap()).unwrap();
        let shards = assign(GROUPS, &ws).unwrap();
        let mut tested: Vec<&str> = shards.iter().flat_map(crates).collect();
        tested.sort_unstable();
        let mut all: Vec<&str> = ws.iter().map(|m| m.name.as_str()).collect();
        all.sort_unstable();
        assert_eq!(tested, all);
        assert_eq!(shards.iter().filter(|s| **s == Shard::Checks).count(), 1);
    }

    #[test]
    fn a_feature_only_another_shard_turns_on_is_a_difference() {
        let ws = [m("lib"), m("geom")];
        let workspace = "\
lib v0.1.0 (/w/lib)|default,synth
geom v0.1.0 (/w/geom)|
geom v0.18.0 (https://github.com/x/y?rev=1#1)|old
lib v0.1.0 (/w/lib)|default,synth (*)
";
        let alone = "\
lib v0.1.0 (/w/lib)|default
geom v0.1.0 (/w/geom)|
";
        let workspace = tree_features(workspace, &ws);
        assert_eq!(workspace["lib"], BTreeSet::from(["default,synth".to_owned()]));
        assert_eq!(workspace["geom"], BTreeSet::from([String::new()]));
        let alone = tree_features(alone, &ws);
        assert_eq!(feature_differences(4, &ws, &workspace, &alone), ["lib: features [default] in shard 4, [default,synth] with the whole workspace"]);
        assert!(feature_differences(4, &ws, &workspace, &workspace).is_empty());
        assert_eq!(
            feature_differences(4, &[m("lib")], &workspace, &BTreeMap::new()),
            ["lib: features absent in shard 4, [default,synth] with the whole workspace"]
        );
    }
}
