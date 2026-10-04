//! Port of `tree.ts`: turns the session list and the stored tree prefs into the rows of the
//! sessions panel, plus the displayed orders that reordering and freezing work from.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use indexmap::IndexMap;
use sdeck_core::discovery::git_status::GitStatus;
use sdeck_core::status::session_usage::RateLimitUsage;
use sdeck_core::status::usage_display::UsageMetric;
use sdeck_core::store::tree_prefs::{
    arrange_by_manual_order, arrange_projects, folder_node_key, project_node_key, sort_sessions, GroupView,
    SessionCategory, SessionPin, TreePrefs,
};

use crate::sessions::DeckSession;

#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Folder {
        folder_id: String,
        name: String,
        count: usize,
        running: usize,
        waiting: usize,
        collapsed: bool,
        hotkey: Option<u8>,
    },
    Project {
        /// The folder-path key prefs (pins, folders, order, collapse) are stored under.
        project_key: String,
        /// Unique per row: equals `project_key` unless one project is split per account.
        node_id: String,
        label: String,
        root: String,
        count: usize,
        running: usize,
        waiting: usize,
        collapsed: bool,
        /// 1 inside a folder.
        depth: usize,
        hotkey: Option<u8>,
        /// Worst case across the project's visible sessions (they usually share a cwd).
        git: Option<GitStatus>,
    },
    Session {
        uid: u64,
        is_last: bool,
        depth: usize,
        pin: Option<SessionPin>,
    },
    Divider {
        label: String,
    },
    Tag {
        name: String,
        count: usize,
    },
    Usage {
        metric: UsageMetric,
        percent: Option<f64>,
        updated_at: Option<i64>,
        reset_label: Option<String>,
    },
    /// The selected session's model and effort, e.g. `Opus 5.5 · high`.
    UsageModel {
        label: Option<String>,
    },
}

/// The rows under the USAGE divider.
pub fn usage_rows(usage: &UsageSectionInput) -> Vec<TreeRow> {
    let rate = usage.rate_limit.as_ref();
    let model_label = usage.model.as_ref().map(|model| match &usage.effort {
        Some(effort) => format!("{model} · {effort}"),
        None => model.clone(),
    });
    vec![
        TreeRow::UsageModel { label: model_label },
        TreeRow::Usage {
            metric: UsageMetric::Context,
            percent: usage.context_percent,
            updated_at: usage.context_updated_at,
            reset_label: usage
                .context_percent
                .zip(usage.context_startup_percent)
                .and_then(|(now, startup)| (now > startup).then(|| format!("{}% startup", startup.round()))),
        },
        TreeRow::Usage {
            metric: UsageMetric::FiveHour,
            percent: rate.and_then(|r| r.five_hour_percent),
            updated_at: rate.map(|r| r.updated_at),
            reset_label: usage.five_hour_reset_label.clone(),
        },
        TreeRow::Usage {
            metric: UsageMetric::SevenDay,
            percent: rate.and_then(|r| r.seven_day_percent),
            updated_at: rate.map(|r| r.updated_at),
            reset_label: usage.seven_day_reset_label.clone(),
        },
    ]
}

impl TreeRow {
    /// Dividers and the usage block are never selected.
    pub fn is_unselectable(&self) -> bool {
        matches!(
            self,
            Self::Divider { .. } | Self::Usage { .. } | Self::UsageModel { .. }
        )
    }
}

/// The tree node a session belongs to: its project, or its project's per-account node when projects
/// are `split` per account.
pub fn node_id_of(s: &DeckSession, split: bool) -> String {
    match (&s.account, split) {
        (Some(a), true) => format!("{}\u{0}{}", s.project_key, a.config_dir.to_string_lossy()),
        _ => s.project_key.clone(),
    }
}

/// Input for the bottom usage section; `TreeOptions::usage == None` hides the whole section.
/// Context is specific to the selected Claude session's transcript; `rate_limit` (5h/7d) is
/// account-wide.
#[derive(Debug, Clone, Default)]
pub struct UsageSectionInput {
    pub context_percent: Option<f64>,
    pub context_updated_at: Option<i64>,
    /// The session's first context reading, shown beside the current one.
    pub context_startup_percent: Option<f64>,
    /// The session's model display name and reasoning effort.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub rate_limit: Option<RateLimitUsage>,
    pub five_hour_reset_label: Option<String>,
    pub seven_day_reset_label: Option<String>,
}

/// A per-session predicate or lookup the tree is built from.
type Per<'a, T> = Box<dyn Fn(&DeckSession) -> T + 'a>;

pub struct TreeOptions<'a> {
    pub include: Per<'a, bool>,
    pub category_of: Per<'a, SessionCategory>,
    /// True for a finished-but-unseen session: `category_of` folds it into waiting for sort/filter
    /// purposes, but badges and counts must show it as done, not waiting.
    pub is_done_unseen: Per<'a, bool>,
    pub pin_of: Per<'a, Option<SessionPin>>,
    /// `None` when `ui.gitStatus` is off, or the cwd isn't a git repo (or hasn't been polled yet).
    pub git_of: Per<'a, Option<GitStatus>>,
    /// Sessions that exist for the tree at all (not in a hidden project); the tag counts use these.
    pub listed: Per<'a, bool>,
    /// A status/time filter is on: groups with nothing visible are hidden, even empty folders.
    pub filtering: bool,
    /// Projects never moved sort by most-recent activity when true, alphabetically when false.
    pub recent_projects_first: bool,
    /// Sessions sort by activity when true, or keep a manual order when false.
    pub recent_sessions_first: bool,
    pub tags_of: Per<'a, Vec<String>>,
    pub usage: Option<UsageSectionInput>,
    /// `accounts.shareProjects`: one project node for a folder used by several accounts (true), or
    /// one node per account, labelled with the account's email (false).
    pub share_projects: bool,
    /// `accounts.showAllSessions`: every account's sessions (true), or only the active account's.
    pub show_all_sessions: bool,
    /// The config dir of the active account (the one new sessions launch as); `None` = unknown.
    pub active_account_dir: Option<PathBuf>,
}

impl TreeOptions<'_> {
    /// Everything shown, nothing pinned, no usage block: the starting point callers override.
    pub fn new() -> Self {
        TreeOptions {
            include: Box::new(|_| true),
            category_of: Box::new(|_| SessionCategory::Idle),
            is_done_unseen: Box::new(|_| false),
            pin_of: Box::new(|_| None),
            git_of: Box::new(|_| None),
            listed: Box::new(|_| true),
            filtering: false,
            recent_projects_first: false,
            recent_sessions_first: true,
            tags_of: Box::new(|_| Vec::new()),
            usage: None,
            share_projects: true,
            show_all_sessions: true,
            active_account_dir: None,
        }
    }
}

impl Default for TreeOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
pub struct BuiltTree {
    pub rows: Vec<TreeRow>,
    /// Displayed project order per container (`""` = top level, else folder id), for reordering.
    pub containers: IndexMap<String, Vec<String>>,
    /// Displayed session order per project key, for reordering. A project split per account holds
    /// both nodes' sessions under its one key.
    pub session_containers: IndexMap<String, Vec<String>>,
}

pub fn is_active(c: SessionCategory) -> bool {
    matches!(c, SessionCategory::Running | SessionCategory::Waiting)
}

/// Running, waiting (which already folds in "finished, not seen yet") or idle: anything actually
/// started here, as opposed to stopped or erroring. Used by `[`/`]`.
pub fn is_started(c: SessionCategory) -> bool {
    matches!(
        c,
        SessionCategory::Running | SessionCategory::Waiting | SessionCategory::Idle
    )
}

/// One badge for the whole project: worst (highest) ahead/behind/dirty across its sessions.
fn aggregate_git(statuses: impl Iterator<Item = Option<GitStatus>>) -> Option<GitStatus> {
    statuses.flatten().fold(None, |acc: Option<GitStatus>, g| {
        Some(match acc {
            None => GitStatus { branch: None, ..g },
            Some(a) => GitStatus {
                branch: None,
                ahead: a.ahead.max(g.ahead),
                behind: a.behind.max(g.behind),
                dirty: a.dirty.max(g.dirty),
            },
        })
    })
}

fn is_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

/// The last path component, with either separator (project roots are Windows paths, but tests and
/// other platforms mix them).
fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(is_separator);
    trimmed.rsplit(is_separator).next().unwrap_or(trimmed)
}

fn parent_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(is_separator);
    match trimmed.rfind(is_separator) {
        Some(i) => base_name(&trimmed[..i]),
        None => "",
    }
}

/// The folder name, plus its parent's name (`acme-storefront · demo-projects`) when two projects
/// share a name.
pub fn project_labels(roots: &[String]) -> HashMap<String, String> {
    let base = |root: &str| {
        let name = base_name(root);
        if name.is_empty() {
            root.to_string()
        } else {
            name.to_string()
        }
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for root in roots {
        *counts.entry(base(root).to_lowercase()).or_default() += 1;
    }
    roots
        .iter()
        .map(|root| {
            let duplicated = counts.get(&base(root).to_lowercase()).copied().unwrap_or(0) > 1;
            let label = if duplicated {
                format!("{} · {}", base(root), parent_name(root))
            } else {
                base(root)
            };
            (root.clone(), label)
        })
        .collect()
}

/// Pinned bands as in `sort_sessions`, but the rest keeps `manual_order` instead of sorting by
/// recency. A session just started via "new session" (`pending_top_order`) renders at the very
/// front of the unpinned band right away, before its id is even known. Any other session not yet in
/// `manual_order` is appended, most recent first.
fn sort_sessions_manual<'a>(
    items: &[&'a DeckSession],
    manual_order: &[String],
    pin_of: &dyn Fn(&DeckSession) -> Option<SessionPin>,
) -> Vec<&'a DeckSession> {
    let by_recent = |a: &&DeckSession, b: &&DeckSession| b.mtime_ms.cmp(&a.mtime_ms);
    let sorted = |mut list: Vec<&'a DeckSession>| {
        list.sort_by(by_recent);
        list
    };
    let pinned = |pin: SessionPin| sorted(items.iter().copied().filter(|i| pin_of(i) == Some(pin)).collect());
    let rest: Vec<&DeckSession> = items.iter().copied().filter(|i| pin_of(i).is_none()).collect();
    let pending_top = sorted(rest.iter().copied().filter(|s| s.pending_top_order).collect());
    let remaining: Vec<&DeckSession> = rest.iter().copied().filter(|s| !s.pending_top_order).collect();
    let identified = sorted(remaining.iter().copied().filter(|s| s.id.is_some()).collect());
    let unidentified = sorted(remaining.iter().copied().filter(|s| s.id.is_none()).collect());
    let by_id: HashMap<&str, &DeckSession> = identified
        .iter()
        .map(|s| (s.id.as_deref().unwrap_or_default(), *s))
        .collect();
    let ids: Vec<String> = identified.iter().filter_map(|s| s.id.clone()).collect();
    let ordered = arrange_by_manual_order(&ids, manual_order);

    let mut out = pinned(SessionPin::Top);
    out.extend(pending_top);
    out.extend(ordered.iter().filter_map(|id| by_id.get(id.as_str()).copied()));
    out.extend(unidentified);
    out.extend(pinned(SessionPin::Bottom));
    out
}

struct Bucket<'a> {
    /// The key its prefs are stored under (`normalize_fs_path` of the git root).
    key: String,
    /// Unique per node: `key`, plus the account when the project is split per account.
    node_id: String,
    root: String,
    /// The account suffix of a split node's label.
    account_label: Option<String>,
    all: Vec<&'a DeckSession>,
    visible: Vec<&'a DeckSession>,
}

fn account_label(s: &DeckSession) -> Option<String> {
    let account = s.account.as_ref()?;
    Some(account.email.clone().unwrap_or_else(|| {
        account
            .config_dir
            .file_name()
            .map_or_else(|| "default".to_string(), |n| n.to_string_lossy().into_owned())
    }))
}

/// Case-insensitive first, then exact, so ordering doesn't jump around with capitalization.
fn compare_labels(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b))
}

struct Builder<'o> {
    opts: &'o TreeOptions<'o>,
    tree: &'o TreePrefs,
    rows: Vec<TreeRow>,
    session_containers: IndexMap<String, Vec<String>>,
    hotkey: u8,
}

impl Builder<'_> {
    fn next_hotkey(&mut self) -> Option<u8> {
        (self.hotkey <= 9).then(|| {
            self.hotkey += 1;
            self.hotkey - 1
        })
    }

    fn counts(&self, list: &[&DeckSession]) -> (usize, usize, usize) {
        let category = |s: &DeckSession| (self.opts.category_of)(s);
        let running = list
            .iter()
            .filter(|s| category(s) == SessionCategory::Running)
            .count();
        let waiting = list
            .iter()
            .filter(|s| category(s) == SessionCategory::Waiting && !(self.opts.is_done_unseen)(s))
            .count();
        (list.len(), running, waiting)
    }

    fn emit_project(&mut self, b: &Bucket, label: &str, depth: usize) {
        let collapsed = self.tree.collapsed.contains(&project_node_key(&b.key));
        let (count, running, waiting) = self.counts(&b.visible);
        let hotkey = if depth == 0 { self.next_hotkey() } else { None };
        self.rows.push(TreeRow::Project {
            project_key: b.key.clone(),
            node_id: b.node_id.clone(),
            label: label.to_string(),
            root: b.root.clone(),
            count,
            running,
            waiting,
            collapsed,
            depth,
            hotkey,
            git: aggregate_git(b.visible.iter().map(|s| (self.opts.git_of)(s))),
        });
        if collapsed {
            return;
        }
        let ordered: Vec<&DeckSession> = if self.opts.recent_sessions_first {
            sort_sessions(
                &b.visible,
                self.tree.sort,
                |s| (self.opts.pin_of)(s),
                |s| s.mtime_ms,
                |s| (self.opts.category_of)(s),
            )
        } else {
            let manual = self.tree.session_order.get(&b.key).map_or(&[][..], Vec::as_slice);
            sort_sessions_manual(&b.visible, manual, &*self.opts.pin_of)
        };
        self.session_containers
            .entry(b.key.clone())
            .or_default()
            .extend(ordered.iter().filter_map(|s| s.id.clone()));
        let last = ordered.len().saturating_sub(1);
        for (i, s) in ordered.iter().enumerate() {
            self.rows.push(TreeRow::Session {
                uid: s.uid,
                is_last: i == last,
                depth: depth + 1,
                pin: (self.opts.pin_of)(s),
            });
        }
    }
}

/// Folders (in their manual order) then top-level projects. Projects never moved default to a fixed
/// alphabetical order, or most recent activity first when `opts.recent_projects_first` is on.
/// Collapsed nodes hide their children. In the "active" view, groups with running/waiting sessions
/// come first, and an "idle / done" divider separates the rest.
pub fn build_tree(sessions: &[DeckSession], tree: &TreePrefs, opts: &TreeOptions) -> BuiltTree {
    let shown: Vec<&DeckSession> = sessions
        .iter()
        .filter(|s| (opts.listed)(s))
        .filter(|s| {
            opts.show_all_sessions
                || match (&s.account, &opts.active_account_dir) {
                    (Some(account), Some(active)) => &account.config_dir == active,
                    _ => true,
                }
        })
        .collect();

    // Buckets, one per node: a project is one node, or one per account when split.
    let mut buckets: Vec<Bucket> = Vec::new();
    let mut bucket_of: HashMap<String, usize> = HashMap::new();
    for &s in &shown {
        let split = !opts.share_projects;
        let node_id = node_id_of(s, split);
        let i = *bucket_of.entry(node_id.clone()).or_insert_with(|| {
            buckets.push(Bucket {
                key: s.project_key.clone(),
                node_id,
                root: s.project_root.clone(),
                account_label: if split { account_label(s) } else { None },
                all: Vec::new(),
                visible: Vec::new(),
            });
            buckets.len() - 1
        });
        buckets[i].all.push(s);
        if (opts.include)(s) {
            buckets[i].visible.push(s);
        }
    }

    // Prefs are per project key: a split project's nodes share one entry, kept together here.
    let mut by_key: IndexMap<String, Vec<usize>> = IndexMap::new();
    for (i, b) in buckets.iter().enumerate() {
        by_key.entry(b.key.clone()).or_default().push(i);
    }
    for nodes in by_key.values_mut() {
        nodes.sort_by(|&a, &b| compare_labels(&split_name(&buckets[a]), &split_name(&buckets[b])));
    }

    let mut unique_roots: Vec<String> = Vec::new();
    for b in &buckets {
        if !unique_roots.contains(&b.root) {
            unique_roots.push(b.root.clone());
        }
    }
    let labels = project_labels(&unique_roots);
    let label_of = |b: &Bucket| {
        let base = labels.get(&b.root).cloned().unwrap_or_else(|| b.root.clone());
        match &b.account_label {
            Some(account) => format!("{base} · {account}"),
            None => base,
        }
    };
    let key_label = |key: &String| {
        let first = &buckets[by_key[key][0]];
        labels
            .get(&first.root)
            .cloned()
            .unwrap_or_else(|| first.root.clone())
    };
    let latest = |key: &String| {
        by_key[key]
            .iter()
            .flat_map(|&i| buckets[i].all.iter().map(|s| s.mtime_ms))
            .max()
            .unwrap_or(0)
    };
    let mut default_order: Vec<String> = by_key.keys().cloned().collect();
    if opts.recent_projects_first {
        default_order.sort_by_key(|k| std::cmp::Reverse(latest(k)));
    } else {
        default_order.sort_by(|a, b| compare_labels(&key_label(a), &key_label(b)));
    }
    let arranged = arrange_projects(tree, &default_order);

    let shown_nodes = |keys: &[String]| -> Vec<usize> {
        keys.iter()
            .filter_map(|k| by_key.get(k))
            .flatten()
            .copied()
            .filter(|&i| !buckets[i].visible.is_empty() || !opts.filtering)
            .collect()
    };
    let unique_keys = |nodes: &[usize]| -> Vec<String> {
        let mut seen = HashSet::new();
        nodes
            .iter()
            .map(|&i| buckets[i].key.clone())
            .filter(|k| seen.insert(k.clone()))
            .collect()
    };
    let project_active = |i: usize| {
        buckets[i]
            .visible
            .iter()
            .any(|s| is_active((opts.category_of)(s)) && !(opts.is_done_unseen)(s))
    };

    struct FolderItem {
        id: String,
        name: String,
        nodes: Vec<usize>,
    }
    let folder_items: Vec<FolderItem> = arranged
        .folders
        .iter()
        .map(|f| FolderItem {
            id: f.folder.id.clone(),
            name: f.folder.name.clone(),
            nodes: shown_nodes(&f.projects),
        })
        .filter(|f| !f.nodes.is_empty() || !opts.filtering)
        .collect();
    let root_nodes = shown_nodes(&arranged.root);

    let mut b = Builder {
        opts,
        tree,
        rows: Vec::new(),
        session_containers: IndexMap::new(),
        hotkey: 1,
    };
    let mut containers: IndexMap<String, Vec<String>> = IndexMap::new();

    let emit_folder = |b: &mut Builder, containers: &mut IndexMap<String, Vec<String>>, f: &FolderItem| {
        let (active, rest): (Vec<usize>, Vec<usize>) = f.nodes.iter().partition(|&&i| project_active(i));
        let ordered: Vec<usize> = if tree.view == GroupView::Active {
            active.into_iter().chain(rest).collect()
        } else {
            f.nodes.clone()
        };
        containers.insert(f.id.clone(), unique_keys(&ordered));
        let collapsed = tree.collapsed.contains(&folder_node_key(&f.id));
        let visible: Vec<&DeckSession> = f
            .nodes
            .iter()
            .flat_map(|&i| buckets[i].visible.iter().copied())
            .collect();
        let (count, running, waiting) = b.counts(&visible);
        let hotkey = b.next_hotkey();
        b.rows.push(TreeRow::Folder {
            folder_id: f.id.clone(),
            name: f.name.clone(),
            count,
            running,
            waiting,
            collapsed,
            hotkey,
        });
        if !collapsed {
            for &i in &ordered {
                b.emit_project(&buckets[i], &label_of(&buckets[i]), 1);
            }
        }
    };

    if tree.view == GroupView::Active {
        let (active_folders, rest_folders): (Vec<&FolderItem>, Vec<&FolderItem>) = folder_items
            .iter()
            .partition(|f| f.nodes.iter().any(|&i| project_active(i)));
        let (active_roots, rest_roots): (Vec<usize>, Vec<usize>) =
            root_nodes.iter().partition(|&&i| project_active(i));
        let all_roots: Vec<usize> = active_roots.iter().chain(&rest_roots).copied().collect();
        containers.insert(String::new(), unique_keys(&all_roots));
        for f in &active_folders {
            emit_folder(&mut b, &mut containers, f);
        }
        for &i in &active_roots {
            b.emit_project(&buckets[i], &label_of(&buckets[i]), 0);
        }
        if (!active_folders.is_empty() || !active_roots.is_empty())
            && (!rest_folders.is_empty() || !rest_roots.is_empty())
        {
            b.rows.push(TreeRow::Divider {
                label: "idle / done".into(),
            });
        }
        for f in &rest_folders {
            emit_folder(&mut b, &mut containers, f);
        }
        for &i in &rest_roots {
            b.emit_project(&buckets[i], &label_of(&buckets[i]), 0);
        }
    } else {
        containers.insert(String::new(), unique_keys(&root_nodes));
        for f in &folder_items {
            emit_folder(&mut b, &mut containers, f);
        }
        for &i in &root_nodes {
            b.emit_project(&buckets[i], &label_of(&buckets[i]), 0);
        }
    }

    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in &shown {
        for tag in (opts.tags_of)(s) {
            *tag_counts.entry(tag).or_default() += 1;
        }
    }
    if !tag_counts.is_empty() {
        b.rows.push(TreeRow::Divider { label: "TAGS".into() });
        let mut tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
        tags.sort_by(|a, b| compare_labels(&a.0, &b.0));
        for (name, count) in tags {
            b.rows.push(TreeRow::Tag { name, count });
        }
    }

    if let Some(usage) = &opts.usage {
        b.rows.push(TreeRow::Divider {
            label: "USAGE".into(),
        });
        b.rows.extend(usage_rows(usage));
    }

    BuiltTree {
        rows: b.rows,
        containers,
        session_containers: b.session_containers,
    }
}

/// What orders a project's per-account nodes against each other.
fn split_name(b: &Bucket) -> String {
    b.account_label.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdeck_core::status::account::Account;
    use sdeck_core::store::tree_prefs::{
        default_tree_prefs, freeze_session_order, prepend_session, FolderPrefs,
    };

    fn session(id: &str, project: &str, mtime: i64) -> DeckSession {
        let root = format!("C:\\repos\\{project}");
        let mut s = DeckSession::new("claude", Some(id), &root, id, mtime);
        s.project_key = root.to_lowercase();
        s
    }

    fn key(project: &str) -> String {
        format!("c:\\repos\\{project}")
    }

    /// Compact picture of the rows: "1F:Work(2)", "   P:web", "    s:w1", "--".
    fn outline(rows: &[TreeRow], sessions: &[DeckSession]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Folder {
                    name,
                    count,
                    collapsed,
                    hotkey,
                    ..
                } => {
                    format!(
                        "{}F:{name}({count}){}",
                        hotkey.map_or(' '.to_string(), |h| h.to_string()),
                        if *collapsed { "+" } else { "" }
                    )
                }
                TreeRow::Project {
                    label,
                    collapsed,
                    depth,
                    hotkey,
                    ..
                } => format!(
                    "{}{}P:{label}{}",
                    hotkey.map_or(' '.to_string(), |h| h.to_string()),
                    "  ".repeat(*depth),
                    if *collapsed { "+" } else { "" }
                ),
                TreeRow::Session { uid, depth, pin, .. } => {
                    let id = sessions.iter().find(|s| s.uid == *uid).and_then(|s| s.id.clone());
                    format!(
                        "{}s:{}{}",
                        "  ".repeat(*depth),
                        id.unwrap_or("null".into()),
                        if pin.is_some() { "^" } else { "" }
                    )
                }
                TreeRow::Tag { name, count } => format!("T:{name}({count})"),
                _ => "--".into(),
            })
            .collect()
    }

    fn fixture() -> Vec<DeckSession> {
        vec![
            session("a1", "api", 50),
            session("a2", "api", 10),
            session("w1", "web", 40),
            session("d1", "docs", 30),
        ]
    }

    fn category(s: &DeckSession) -> SessionCategory {
        match s.id.as_deref() {
            Some("a1") => SessionCategory::Idle,
            Some("w1") => SessionCategory::Running,
            _ => SessionCategory::Stopped,
        }
    }

    fn opts<'a>() -> TreeOptions<'a> {
        TreeOptions {
            category_of: Box::new(category),
            ..TreeOptions::new()
        }
    }

    fn work_tree() -> TreePrefs {
        TreePrefs {
            folders: vec![FolderPrefs {
                id: "f1".into(),
                name: "Work".into(),
                projects: vec![key("web"), key("docs")],
            }],
            ..default_tree_prefs()
        }
    }

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn folders_come_first_then_top_level_projects_numbering_top_level_rows() {
        let sessions = fixture();
        let built = build_tree(&sessions, &work_tree(), &opts());
        assert_eq!(
            outline(&built.rows, &sessions),
            [
                "1F:Work(2)",
                "   P:web",
                "    s:w1",
                "   P:docs",
                "    s:d1",
                "2P:api",
                "  s:a1",
                "  s:a2"
            ]
        );
        assert_eq!(built.containers["f1"], [key("web"), key("docs")]);
        assert_eq!(built.containers[""], [key("api")]);
    }

    #[test]
    fn top_level_projects_default_to_alphabetical_or_recent_first() {
        let sessions = fixture();
        let labels = |o: &TreeOptions| -> Vec<String> {
            build_tree(&sessions, &default_tree_prefs(), o)
                .rows
                .iter()
                .filter_map(|r| match r {
                    TreeRow::Project { label, .. } => Some(label.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(labels(&opts()), ["api", "docs", "web"]);
        let recent = TreeOptions {
            recent_projects_first: true,
            ..opts()
        };
        assert_eq!(labels(&recent), ["api", "web", "docs"]);
    }

    #[test]
    fn a_fixed_session_order_moves_only_via_the_stored_session_order() {
        let sessions = fixture();
        let mut tree = default_tree_prefs();
        tree.session_order.insert(key("api"), strs(&["a2", "a1"]));
        let o = TreeOptions {
            recent_sessions_first: false,
            ..opts()
        };
        let built = build_tree(&sessions, &tree, &o);
        assert_eq!(
            outline(&built.rows, &sessions)[..3],
            ["1P:api", "  s:a2", "  s:a1"]
        );
        assert_eq!(built.session_containers[&key("api")], ["a2", "a1"]);
        assert_eq!(built.session_containers[&key("docs")], ["d1"]);
    }

    #[test]
    fn with_a_fixed_order_a_session_that_becomes_newest_does_not_jump_once_frozen() {
        let mut tree = default_tree_prefs();
        let o = TreeOptions {
            recent_sessions_first: false,
            ..opts()
        };
        let render_once = |tree: &mut TreePrefs, sessions: &[DeckSession]| {
            let built = build_tree(sessions, tree, &o);
            let displayed: Vec<_> = built
                .session_containers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            *tree = freeze_session_order(tree, &displayed);
            outline(&built.rows, sessions)
        };
        assert_eq!(
            render_once(&mut tree, &fixture())[..3],
            ["1P:api", "  s:a1", "  s:a2"]
        );
        let bumped = vec![
            session("a1", "api", 50),
            session("a2", "api", 999),
            session("w1", "web", 40),
            session("d1", "docs", 30),
        ];
        assert_eq!(
            render_once(&mut tree, &bumped)[..3],
            ["1P:api", "  s:a1", "  s:a2"]
        );
    }

    #[test]
    fn a_prepended_session_renders_first_and_stays_first_once_frozen() {
        let mut tree = default_tree_prefs();
        tree.session_order.insert(key("api"), strs(&["a2", "a1"]));
        tree = prepend_session(&tree, &key("api"), "a3");
        let mut sessions = fixture();
        sessions.push(session("a3", "api", 5));
        let o = TreeOptions {
            recent_sessions_first: false,
            ..opts()
        };
        let built = build_tree(&sessions, &tree, &o);
        assert_eq!(
            outline(&built.rows, &sessions)[..4],
            ["1P:api", "  s:a3", "  s:a2", "  s:a1"]
        );
        let displayed: Vec<_> = built
            .session_containers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        tree = freeze_session_order(&tree, &displayed);
        let again = build_tree(&sessions, &tree, &o);
        assert_eq!(
            outline(&again.rows, &sessions)[..4],
            ["1P:api", "  s:a3", "  s:a2", "  s:a1"]
        );
    }

    #[test]
    fn a_pending_top_order_session_renders_on_top_before_it_has_an_id() {
        let mut tree = default_tree_prefs();
        tree.session_order.insert(key("api"), strs(&["a2", "a1"]));
        let mut fresh = DeckSession::new("claude", None, &key("api"), "(new session)", 1_000_000);
        fresh.pending_top_order = true;
        let mut sessions = fixture();
        sessions.push(fresh);
        let o = TreeOptions {
            recent_sessions_first: false,
            ..opts()
        };
        let built = build_tree(&sessions, &tree, &o);
        assert_eq!(
            outline(&built.rows, &sessions)[..4],
            ["1P:api", "  s:null", "  s:a2", "  s:a1"]
        );
        assert_eq!(built.session_containers[&key("api")], ["a2", "a1"]);
    }

    #[test]
    fn collapsed_folders_and_projects_hide_their_children() {
        let sessions = fixture();
        let mut tree = work_tree();
        tree.collapsed = vec![folder_node_key("f1"), project_node_key(&key("api"))];
        assert_eq!(
            outline(&build_tree(&sessions, &tree, &opts()).rows, &sessions),
            ["1F:Work(2)+", "2P:api+"]
        );
    }

    #[test]
    fn a_filter_hides_projects_and_folders_with_nothing_visible() {
        let sessions = fixture();
        let mut tree = work_tree();
        tree.folders.push(FolderPrefs {
            id: "f2".into(),
            name: "Empty".into(),
            projects: vec![],
        });
        assert_eq!(
            outline(&build_tree(&sessions, &tree, &opts()).rows, &sessions)[5],
            "2F:Empty(0)"
        );
        let running = TreeOptions {
            include: Box::new(|s| category(s) == SessionCategory::Running),
            filtering: true,
            ..opts()
        };
        assert_eq!(
            outline(&build_tree(&sessions, &tree, &running).rows, &sessions),
            ["1F:Work(1)", "   P:web", "    s:w1"]
        );
    }

    #[test]
    fn the_active_view_hoists_running_and_waiting_groups_above_an_idle_done_divider() {
        let sessions = fixture();
        let tree = TreePrefs {
            view: GroupView::Active,
            ..default_tree_prefs()
        };
        assert_eq!(
            outline(&build_tree(&sessions, &tree, &opts()).rows, &sessions),
            ["1P:web", "  s:w1", "--", "2P:api", "  s:a1", "  s:a2", "3P:docs", "  s:d1"]
        );
    }

    #[test]
    fn done_unseen_sessions_count_as_done_not_waiting_in_badges() {
        let sessions = vec![session("a1", "api", 50), session("a2", "api", 10)];
        let o = TreeOptions {
            category_of: Box::new(|_| SessionCategory::Waiting),
            is_done_unseen: Box::new(|s| s.id.as_deref() == Some("a1")),
            ..TreeOptions::new()
        };
        let built = build_tree(&sessions, &default_tree_prefs(), &o);
        match &built.rows[0] {
            TreeRow::Project {
                count,
                running,
                waiting,
                ..
            } => assert_eq!((*count, *running, *waiting), (2, 0, 1)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pinned_sessions_sit_around_the_rest() {
        let sessions = fixture();
        let o = TreeOptions {
            pin_of: Box::new(|s| (s.id.as_deref() == Some("a2")).then_some(SessionPin::Top)),
            ..opts()
        };
        assert_eq!(
            outline(&build_tree(&sessions, &default_tree_prefs(), &o).rows, &sessions)[..3],
            ["1P:api", "  s:a2^", "  s:a1"]
        );
    }

    #[test]
    fn git_status_aggregates_to_the_worst_case_per_project() {
        let sessions = fixture();
        let o = TreeOptions {
            git_of: Box::new(|s| match s.id.as_deref() {
                Some("a1") => Some(GitStatus {
                    branch: Some("main".into()),
                    ahead: 0,
                    behind: 0,
                    dirty: 0,
                }),
                Some("a2") => Some(GitStatus {
                    branch: Some("main".into()),
                    ahead: 2,
                    behind: 0,
                    dirty: 5,
                }),
                _ => None,
            }),
            ..opts()
        };
        let built = build_tree(&sessions, &default_tree_prefs(), &o);
        let git_of = |name: &str| {
            built.rows.iter().find_map(|r| match r {
                TreeRow::Project { label, git, .. } if label == name => Some(git.clone()),
                _ => None,
            })
        };
        let worst = git_of("api").unwrap().unwrap();
        assert_eq!((worst.ahead, worst.behind, worst.dirty), (2, 0, 5));
        assert_eq!(git_of("docs"), Some(None));
    }

    #[test]
    fn a_tags_section_counts_sessions_per_tag_and_is_omitted_when_nothing_is_tagged() {
        let sessions = fixture();
        let o = TreeOptions {
            tags_of: Box::new(|s| match s.id.as_deref() {
                Some("a1") => strs(&["urgent", "vwde"]),
                Some("w1") => strs(&["vwde"]),
                _ => vec![],
            }),
            ..opts()
        };
        let rows = build_tree(&sessions, &default_tree_prefs(), &o).rows;
        let tail: Vec<_> = rows
            .iter()
            .filter(|r| {
                matches!(r, TreeRow::Tag { .. }) || matches!(r, TreeRow::Divider { label } if label == "TAGS")
            })
            .cloned()
            .collect();
        assert_eq!(
            tail,
            [
                TreeRow::Divider { label: "TAGS".into() },
                TreeRow::Tag {
                    name: "urgent".into(),
                    count: 1
                },
                TreeRow::Tag {
                    name: "vwde".into(),
                    count: 2
                },
            ]
        );
        let plain = build_tree(&sessions, &default_tree_prefs(), &opts()).rows;
        assert!(!plain.iter().any(|r| matches!(r, TreeRow::Tag { .. })));
    }

    #[test]
    fn the_usage_section_is_omitted_unless_asked_for() {
        let sessions = fixture();
        let rows = build_tree(&sessions, &default_tree_prefs(), &opts()).rows;
        assert!(!rows
            .iter()
            .any(|r| matches!(r, TreeRow::Usage { .. } | TreeRow::UsageModel { .. })));
    }

    #[test]
    fn the_usage_section_carries_context_and_rate_limit_data_separately() {
        let sessions = fixture();
        let o = TreeOptions {
            usage: Some(UsageSectionInput {
                context_percent: Some(42.0),
                context_updated_at: Some(789),
                context_startup_percent: Some(6.4),
                model: Some("Opus 5.5".into()),
                effort: Some("high".into()),
                rate_limit: Some(RateLimitUsage {
                    five_hour_percent: Some(30.0),
                    five_hour_resets_at: None,
                    seven_day_percent: Some(75.0),
                    seven_day_resets_at: None,
                    updated_at: 456,
                }),
                five_hour_reset_label: Some("8:30 PM".into()),
                seven_day_reset_label: Some("Sun 4:21 PM".into()),
            }),
            ..opts()
        };
        let rows = build_tree(&sessions, &default_tree_prefs(), &o).rows;
        assert_eq!(
            rows[rows.len() - 5..],
            [
                TreeRow::Divider {
                    label: "USAGE".into()
                },
                TreeRow::UsageModel {
                    label: Some("Opus 5.5 · high".into())
                },
                TreeRow::Usage {
                    metric: UsageMetric::Context,
                    percent: Some(42.0),
                    updated_at: Some(789),
                    reset_label: Some("6% startup".into())
                },
                TreeRow::Usage {
                    metric: UsageMetric::FiveHour,
                    percent: Some(30.0),
                    updated_at: Some(456),
                    reset_label: Some("8:30 PM".into())
                },
                TreeRow::Usage {
                    metric: UsageMetric::SevenDay,
                    percent: Some(75.0),
                    updated_at: Some(456),
                    reset_label: Some("Sun 4:21 PM".into())
                },
            ]
        );
    }

    #[test]
    fn the_usage_section_renders_empty_when_nothing_was_ever_recorded() {
        let sessions = fixture();
        let o = TreeOptions {
            usage: Some(UsageSectionInput::default()),
            ..opts()
        };
        let rows = build_tree(&sessions, &default_tree_prefs(), &o).rows;
        let tail = &rows[rows.len() - 5..];
        assert!(matches!(tail[0], TreeRow::Divider { .. }));
        assert_eq!(tail[1], TreeRow::UsageModel { label: None });
        for r in &tail[2..5] {
            assert!(matches!(r, TreeRow::Usage { percent: None, .. }));
        }
    }

    #[test]
    fn project_labels_add_the_parent_folder_only_when_two_projects_share_a_name() {
        let labels = project_labels(&strs(&["C:\\repos\\api", "X:\\demo\\shop", "C:\\work\\shop"]));
        assert_eq!(labels["C:\\repos\\api"], "api");
        assert_eq!(labels["X:\\demo\\shop"], "shop · demo");
        assert_eq!(labels["C:\\work\\shop"], "shop · work");
    }

    fn account(dir: &str, email: &str) -> Account {
        Account {
            config_dir: PathBuf::from(dir),
            email: Some(email.into()),
            is_default: dir.ends_with(".claude"),
        }
    }

    /// One folder under two accounts, plus a project only the second account has.
    fn two_accounts() -> (Vec<DeckSession>, Account, Account) {
        let (work, me) = (
            account("C:\\u\\.claude", "work@x.com"),
            account("C:\\u\\.claude-me", "me@x.com"),
        );
        let mut sessions = vec![
            session("w-api", "api", 50),
            session("m-api", "api", 40),
            session("m-web", "web", 30),
        ];
        sessions[0].account = Some(work.clone());
        sessions[1].account = Some(me.clone());
        sessions[2].account = Some(me.clone());
        (sessions, work, me)
    }

    #[test]
    fn the_same_folder_under_two_accounts_is_one_node_when_projects_are_shared() {
        let (sessions, _, _) = two_accounts();
        let built = build_tree(&sessions, &default_tree_prefs(), &opts());
        assert_eq!(
            outline(&built.rows, &sessions),
            ["1P:api", "  s:w-api", "  s:m-api", "2P:web", "  s:m-web"]
        );
    }

    #[test]
    fn the_same_folder_splits_into_labelled_nodes_per_account_when_projects_are_not_shared() {
        let (sessions, _, _) = two_accounts();
        let o = TreeOptions {
            share_projects: false,
            ..opts()
        };
        let built = build_tree(&sessions, &default_tree_prefs(), &o);
        assert_eq!(
            outline(&built.rows, &sessions),
            [
                "1P:api · me@x.com",
                "  s:m-api",
                "2P:api · work@x.com",
                "  s:w-api",
                "3P:web · me@x.com",
                "  s:m-web"
            ]
        );
        // Both nodes are one project to the prefs: one container entry, one shared order.
        assert_eq!(built.containers[""], [key("api"), key("web")]);
        assert_eq!(built.session_containers[&key("api")], ["m-api", "w-api"]);
        let node_ids: Vec<_> = built
            .rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::Project {
                    node_id, project_key, ..
                } => Some((node_id.clone(), project_key.clone())),
                _ => None,
            })
            .collect();
        assert_ne!(node_ids[0].0, node_ids[1].0);
        assert_eq!(node_ids[0].1, node_ids[1].1);
    }

    #[test]
    fn hiding_other_accounts_keeps_only_the_active_accounts_sessions_and_projects() {
        let (sessions, work, _) = two_accounts();
        let o = TreeOptions {
            show_all_sessions: false,
            active_account_dir: Some(work.config_dir.clone()),
            ..opts()
        };
        assert_eq!(
            outline(&build_tree(&sessions, &default_tree_prefs(), &o).rows, &sessions),
            ["1P:api", "  s:w-api"]
        );
    }

    #[test]
    fn a_pin_on_the_folder_applies_to_both_split_nodes() {
        let (sessions, _, _) = two_accounts();
        let mut tree = work_tree();
        tree.folders[0].projects = vec![key("api")];
        let o = TreeOptions {
            share_projects: false,
            ..opts()
        };
        let built = build_tree(&sessions, &tree, &o);
        assert_eq!(
            outline(&built.rows, &sessions)[..5],
            [
                "1F:Work(2)",
                "   P:api · me@x.com",
                "    s:m-api",
                "   P:api · work@x.com",
                "    s:w-api"
            ]
        );
        let mut collapsed = tree.clone();
        collapsed.collapsed = vec![project_node_key(&key("api"))];
        let rows = build_tree(&sessions, &collapsed, &o).rows;
        assert_eq!(
            outline(&rows, &sessions)[..3],
            ["1F:Work(2)", "   P:api · me@x.com+", "   P:api · work@x.com+"]
        );
    }

    #[test]
    fn accountless_sessions_survive_hiding_and_splitting() {
        let (mut sessions, work, _) = two_accounts();
        let mut copilot = DeckSession::new("copilot", Some("cp"), "C:\\repos\\api", "cp", 20);
        copilot.project_key = key("api");
        sessions.push(copilot);
        let o = TreeOptions {
            share_projects: false,
            show_all_sessions: false,
            active_account_dir: Some(work.config_dir),
            ..opts()
        };
        assert_eq!(
            outline(&build_tree(&sessions, &default_tree_prefs(), &o).rows, &sessions),
            ["1P:api", "  s:cp", "2P:api · work@x.com", "  s:w-api"]
        );
    }
}
