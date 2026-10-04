use std::collections::HashSet;

use indexmap::IndexMap;
use serde_json::{Map, Value};

/// How sessions are organized. Projects are identified by their key (`normalize_fs_path` of the
/// project's git root). Folders are one level deep and hold projects only.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderPrefs {
    pub id: String,
    pub name: String,
    /// Member project keys, in display order.
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSort {
    Recent,
    Actionable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupView {
    Normal,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPin {
    Top,
    Bottom,
}

impl SessionPin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

/// Same categories the terminal UI's filter pills use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCategory {
    Running,
    Waiting,
    Idle,
    Error,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreePrefs {
    /// In display order, shown before top-level projects.
    pub folders: Vec<FolderPrefs>,
    /// Manual order of top-level projects; projects not listed follow in the default order.
    pub root_order: Vec<String>,
    /// Manual order of each project's sessions, used only when `ui.recentSessionsFirst` is off.
    pub session_order: IndexMap<String, Vec<String>>,
    /// Collapsed folder/project nodes, see `folder_node_key` / `project_node_key`.
    pub collapsed: Vec<String>,
    pub sort: SessionSort,
    pub view: GroupView,
}

impl Default for TreePrefs {
    fn default() -> Self {
        TreePrefs {
            folders: vec![],
            root_order: vec![],
            session_order: IndexMap::new(),
            collapsed: vec![],
            sort: SessionSort::Recent,
            view: GroupView::Normal,
        }
    }
}

pub fn default_tree_prefs() -> TreePrefs {
    TreePrefs::default()
}

pub fn folder_node_key(folder_id: &str) -> String {
    format!("folder:{folder_id}")
}

pub fn project_node_key(project_key: &str) -> String {
    format!("project:{project_key}")
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn unique(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Tolerant: invalid parts fall back to defaults, and a project listed in two places keeps its first one.
pub fn parse_tree_prefs(raw: &Value) -> Option<TreePrefs> {
    let r = raw.as_object()?;
    let mut tree = TreePrefs::default();
    let mut placed: HashSet<String> = HashSet::new();
    for f in r.get("folders").and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = f.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = f.get("name").and_then(Value::as_str) else {
            continue;
        };
        if name.trim().is_empty() || tree.folders.iter().any(|x| x.id == id) {
            continue;
        }
        let members: Vec<String> = string_array(f.get("projects"))
            .into_iter()
            .filter(|p| !placed.contains(p))
            .collect();
        placed.extend(members.iter().cloned());
        tree.folders.push(FolderPrefs {
            id: id.to_string(),
            name: name.to_string(),
            projects: members,
        });
    }
    tree.root_order = string_array(r.get("rootOrder"))
        .into_iter()
        .filter(|p| !placed.contains(p))
        .collect();
    if let Some(orders) = r.get("sessionOrder").and_then(Value::as_object) {
        for (k, v) in orders {
            let arr = string_array(Some(v));
            if !arr.is_empty() {
                tree.session_order.insert(k.clone(), arr);
            }
        }
    }
    tree.collapsed = unique(string_array(r.get("collapsed")));
    match r.get("sort").and_then(Value::as_str) {
        Some("recent") => tree.sort = SessionSort::Recent,
        Some("actionable") => tree.sort = SessionSort::Actionable,
        _ => {}
    }
    match r.get("view").and_then(Value::as_str) {
        Some("normal") => tree.view = GroupView::Normal,
        Some("active") => tree.view = GroupView::Active,
        _ => {}
    }
    Some(tree)
}

pub fn tree_prefs_to_value(tree: &TreePrefs) -> Value {
    let strings = |v: &[String]| -> Value { v.iter().cloned().map(Value::String).collect() };
    let mut obj = Map::new();
    obj.insert(
        "folders".into(),
        tree.folders
            .iter()
            .map(|f| {
                let mut m = Map::new();
                m.insert("id".into(), f.id.clone().into());
                m.insert("name".into(), f.name.clone().into());
                m.insert("projects".into(), strings(&f.projects));
                Value::Object(m)
            })
            .collect(),
    );
    obj.insert("rootOrder".into(), strings(&tree.root_order));
    obj.insert(
        "sessionOrder".into(),
        Value::Object(
            tree.session_order
                .iter()
                .map(|(k, v)| (k.clone(), strings(v)))
                .collect(),
        ),
    );
    obj.insert("collapsed".into(), strings(&tree.collapsed));
    obj.insert(
        "sort".into(),
        match tree.sort {
            SessionSort::Recent => "recent",
            SessionSort::Actionable => "actionable",
        }
        .into(),
    );
    obj.insert(
        "view".into(),
        match tree.view {
            GroupView::Normal => "normal",
            GroupView::Active => "active",
        }
        .into(),
    );
    Value::Object(obj)
}

pub fn is_default_tree(tree: &TreePrefs) -> bool {
    *tree == TreePrefs::default()
}

// ---------------------------------------------------------------------------------------------
// Pure operations: each returns a new TreePrefs
// ---------------------------------------------------------------------------------------------

pub fn create_folder(tree: &TreePrefs, name: &str) -> (TreePrefs, String) {
    let folder_id: String = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();
    let mut next = tree.clone();
    next.folders.push(FolderPrefs {
        id: folder_id.clone(),
        name: name.trim().to_string(),
        projects: vec![],
    });
    (next, folder_id)
}

pub fn rename_folder(tree: &TreePrefs, folder_id: &str, name: &str) -> TreePrefs {
    let mut next = tree.clone();
    for f in next.folders.iter_mut().filter(|f| f.id == folder_id) {
        f.name = name.trim().to_string();
    }
    next
}

/// Its projects move back to the top level, ahead of the other ordered ones.
pub fn delete_folder(tree: &TreePrefs, folder_id: &str) -> TreePrefs {
    let Some(folder) = tree.folders.iter().find(|f| f.id == folder_id) else {
        return tree.clone();
    };
    let mut next = tree.clone();
    next.folders.retain(|f| f.id != folder_id);
    next.root_order = folder
        .projects
        .iter()
        .chain(tree.root_order.iter())
        .cloned()
        .collect();
    let key = folder_node_key(folder_id);
    next.collapsed.retain(|k| *k != key);
    next
}

/// Into a folder (appended at its end), or back to the top level with `folder_id` `None`.
pub fn move_project_to_folder(tree: &TreePrefs, project_key: &str, folder_id: Option<&str>) -> TreePrefs {
    let mut next = tree.clone();
    for f in next.folders.iter_mut() {
        f.projects.retain(|p| p != project_key);
    }
    next.root_order.retain(|p| p != project_key);
    match folder_id {
        None => next.root_order.push(project_key.to_string()),
        Some(id) => match next.folders.iter_mut().find(|f| f.id == id) {
            Some(target) => target.projects.push(project_key.to_string()),
            None => return tree.clone(),
        },
    }
    next
}

pub fn move_folder(tree: &TreePrefs, folder_id: &str, delta: isize) -> TreePrefs {
    let Some(i) = tree.folders.iter().position(|f| f.id == folder_id) else {
        return tree.clone();
    };
    let j = i as isize + delta;
    if j < 0 || j as usize >= tree.folders.len() {
        return tree.clone();
    }
    let mut next = tree.clone();
    next.folders.swap(i, j as usize);
    next
}

/// Appends each displayed key missing from `list`, then swaps `key` with its displayed neighbor
/// `delta` steps away; entries the front end doesn't display keep their stored positions. `None`
/// when the move falls off either end or `key` isn't displayed.
fn swap_with_displayed_neighbor(
    mut list: Vec<String>,
    key: &str,
    delta: isize,
    displayed: &[String],
) -> Option<Vec<String>> {
    for k in displayed {
        if !list.contains(k) {
            list.push(k.clone());
        }
    }
    let visible: Vec<&String> = list.iter().filter(|k| displayed.contains(k)).collect();
    let i = visible.iter().position(|k| k.as_str() == key)? as isize;
    let j = i + delta;
    if j < 0 || j as usize >= visible.len() {
        return None;
    }
    let a = list.iter().position(|k| k == visible[i as usize])?;
    let b = list.iter().position(|k| k == visible[j as usize])?;
    list.swap(a, b);
    Some(list)
}

/// Swaps a project with its displayed neighbor. `displayed` is the container's current order as shown
/// (which may omit projects another view shows, or include ones never ordered): those hidden keep
/// their stored positions, and unordered displayed ones are appended in displayed order first.
pub fn move_project(tree: &TreePrefs, project_key: &str, delta: isize, displayed: &[String]) -> TreePrefs {
    let folder_idx = tree
        .folders
        .iter()
        .position(|f| f.projects.iter().any(|p| p == project_key));
    let list = match folder_idx {
        Some(i) => tree.folders[i].projects.clone(),
        None => tree.root_order.clone(),
    };
    let Some(list) = swap_with_displayed_neighbor(list, project_key, delta, displayed) else {
        return tree.clone();
    };
    let mut next = tree.clone();
    match folder_idx {
        Some(i) => next.folders[i].projects = list,
        None => next.root_order = list,
    }
    next
}

/// Swaps a session with its displayed neighbor within one project. See `move_project`.
pub fn move_session(
    tree: &TreePrefs,
    project_key: &str,
    session_id: &str,
    delta: isize,
    displayed: &[String],
) -> TreePrefs {
    let list = tree.session_order.get(project_key).cloned().unwrap_or_default();
    let Some(list) = swap_with_displayed_neighbor(list, session_id, delta, displayed) else {
        return tree.clone();
    };
    let mut next = tree.clone();
    next.session_order.insert(project_key.to_string(), list);
    next
}

/// With `recentSessionsFirst` off, puts a brand-new session at the very front of its project's manual order.
pub fn prepend_session(tree: &TreePrefs, project_key: &str, session_id: &str) -> TreePrefs {
    let mut list = vec![session_id.to_string()];
    list.extend(
        tree.session_order
            .get(project_key)
            .into_iter()
            .flatten()
            .filter(|id| *id != session_id)
            .cloned(),
    );
    let mut next = tree.clone();
    next.session_order.insert(project_key.to_string(), list);
    next
}

/// Keeps a session's manual position across an id change (e.g. `/clear`): `old_id` stays right after
/// `new_id` so the orphaned transcript lands directly below the live one. A no-op when `old_id` was
/// never recorded, so callers fall back to `prepend_session`.
pub fn rename_session_id(tree: &TreePrefs, project_key: &str, old_id: &str, new_id: &str) -> TreePrefs {
    let Some(list) = tree.session_order.get(project_key) else {
        return tree.clone();
    };
    if !list.iter().any(|id| id == old_id) || old_id == new_id {
        return tree.clone();
    }
    let renamed: Vec<String> = list
        .iter()
        .flat_map(|id| {
            if id == old_id {
                vec![new_id.to_string(), old_id.to_string()]
            } else {
                vec![id.clone()]
            }
        })
        .collect();
    let mut next = tree.clone();
    next.session_order.insert(project_key.to_string(), renamed);
    next
}

/// Locks in each project's currently displayed session order for any session not yet in
/// `session_order`, so it stops shuffling as it becomes active.
pub fn freeze_session_order(tree: &TreePrefs, displayed: &[(String, Vec<String>)]) -> TreePrefs {
    let mut next = tree.clone();
    for (key, ids) in displayed {
        let stored = next.session_order.get(key).cloned().unwrap_or_default();
        let missing: Vec<String> = ids.iter().filter(|id| !stored.contains(id)).cloned().collect();
        if !missing.is_empty() {
            let mut merged = stored;
            merged.extend(missing);
            next.session_order.insert(key.clone(), merged);
        }
    }
    next
}

/// Orders `ids` (already in a fallback order) by `manual`, keeping unlisted ones in that order at the end.
pub fn arrange_by_manual_order(ids: &[String], manual: &[String]) -> Vec<String> {
    let present: HashSet<&String> = ids.iter().collect();
    let ordered: Vec<String> = manual.iter().filter(|id| present.contains(id)).cloned().collect();
    let ordered_set: HashSet<&String> = ordered.iter().collect();
    let rest: Vec<String> = ids
        .iter()
        .filter(|id| !ordered_set.contains(id))
        .cloned()
        .collect();
    ordered.into_iter().chain(rest).collect()
}

pub fn set_collapsed(tree: &TreePrefs, node_key: &str, collapsed: bool) -> TreePrefs {
    let mut next = tree.clone();
    next.collapsed.retain(|k| k != node_key);
    if collapsed {
        next.collapsed.push(node_key.to_string());
    }
    next
}

// ---------------------------------------------------------------------------------------------
// Arranging for display
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ArrangedFolder {
    pub folder: FolderPrefs,
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrangedProjects {
    pub folders: Vec<ArrangedFolder>,
    pub root: Vec<String>,
}

/// Places the projects a front end has (given in its own default order) into folders and top-level
/// order. Folders keep only projects present in `project_keys`; empty folders are still returned.
pub fn arrange_projects(tree: &TreePrefs, project_keys: &[String]) -> ArrangedProjects {
    let present: HashSet<&String> = project_keys.iter().collect();
    let in_folder: HashSet<&String> = tree.folders.iter().flat_map(|f| f.projects.iter()).collect();
    let ordered: Vec<String> = tree
        .root_order
        .iter()
        .filter(|k| present.contains(k) && !in_folder.contains(k))
        .cloned()
        .collect();
    let rest: Vec<String> = project_keys
        .iter()
        .filter(|k| !in_folder.contains(k) && !ordered.contains(k))
        .cloned()
        .collect();
    ArrangedProjects {
        folders: tree
            .folders
            .iter()
            .map(|folder| ArrangedFolder {
                folder: folder.clone(),
                projects: folder
                    .projects
                    .iter()
                    .filter(|k| present.contains(k))
                    .cloned()
                    .collect(),
            })
            .collect(),
        root: ordered.into_iter().chain(rest).collect(),
    }
}

fn actionable_rank(c: SessionCategory) -> u8 {
    match c {
        SessionCategory::Error => 0,
        SessionCategory::Waiting => 1,
        SessionCategory::Running => 2,
        SessionCategory::Idle => 3,
        SessionCategory::Stopped => 4,
    }
}

/// Pinned-to-top first, then the rest by `sort`, then pinned-to-bottom. Pinned bands are newest first.
pub fn sort_sessions<T: Clone>(
    items: &[T],
    sort: SessionSort,
    pin_of: impl Fn(&T) -> Option<SessionPin>,
    mtime_of: impl Fn(&T) -> i64,
    category_of: impl Fn(&T) -> SessionCategory,
) -> Vec<T> {
    let by_recent = |a: &T, b: &T| mtime_of(b).cmp(&mtime_of(a));
    let by_actionable = |a: &T, b: &T| {
        actionable_rank(category_of(a))
            .cmp(&actionable_rank(category_of(b)))
            .then_with(|| by_recent(a, b))
    };
    let band =
        |pin: Option<SessionPin>| -> Vec<T> { items.iter().filter(|i| pin_of(i) == pin).cloned().collect() };
    let mut top = band(Some(SessionPin::Top));
    top.sort_by(by_recent);
    let mut rest = band(None);
    match sort {
        SessionSort::Actionable => rest.sort_by(by_actionable),
        SessionSort::Recent => rest.sort_by(by_recent),
    }
    let mut bottom = band(Some(SessionPin::Bottom));
    bottom.sort_by(by_recent);
    top.into_iter().chain(rest).chain(bottom).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| i.to_string()).collect()
    }

    fn with_folders(names: &[&str]) -> (TreePrefs, Vec<String>) {
        let mut tree = default_tree_prefs();
        let mut ids = vec![];
        for name in names {
            let (t, id) = create_folder(&tree, name);
            tree = t;
            ids.push(id);
        }
        (tree, ids)
    }

    fn orders(pairs: &[(&str, &[&str])]) -> IndexMap<String, Vec<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), s(v))).collect()
    }

    #[test]
    fn parse_falls_back_to_defaults_for_invalid_parts_and_keeps_a_project_in_one_place_only() {
        let tree = parse_tree_prefs(&json!({
            "folders": [
                { "id": "a", "name": "Work", "projects": ["p1", "p2"] },
                { "id": "b", "name": "  ", "projects": ["p3"] },
                { "id": "c", "name": "Home", "projects": ["p2", "p4"] }
            ],
            "rootOrder": ["p1", "p5", 7],
            "sessionOrder": { "p1": ["s2", "s1", 3], "p2": [], "p3": "nope" },
            "sort": "sideways",
            "view": "active"
        }))
        .unwrap();
        assert_eq!(
            tree.folders,
            vec![
                FolderPrefs {
                    id: "a".into(),
                    name: "Work".into(),
                    projects: s(&["p1", "p2"])
                },
                FolderPrefs {
                    id: "c".into(),
                    name: "Home".into(),
                    projects: s(&["p4"])
                },
            ]
        );
        assert_eq!(tree.root_order, s(&["p5"]));
        assert_eq!(tree.session_order, orders(&[("p1", &["s2", "s1"])]));
        assert_eq!(tree.sort, SessionSort::Recent);
        assert_eq!(tree.view, GroupView::Active);
    }

    #[test]
    fn parse_rejects_non_objects_and_round_trips_through_a_value() {
        assert!(parse_tree_prefs(&json!("x")).is_none());
        let (mut tree, ids) = with_folders(&["Work"]);
        tree = move_project_to_folder(&tree, "p1", Some(&ids[0]));
        tree.session_order = orders(&[("p1", &["a", "b"])]);
        tree.collapsed = s(&["project:p1"]);
        tree.sort = SessionSort::Actionable;
        assert_eq!(parse_tree_prefs(&tree_prefs_to_value(&tree)), Some(tree));
    }

    #[test]
    fn move_project_to_folder_moves_between_folders_and_back_to_the_top_level() {
        let (tree, ids) = with_folders(&["Work", "Home"]);
        let mut t = move_project_to_folder(&tree, "p1", Some(&ids[0]));
        t = move_project_to_folder(&t, "p1", Some(&ids[1]));
        assert_eq!(
            t.folders.iter().map(|f| f.projects.clone()).collect::<Vec<_>>(),
            vec![vec![], s(&["p1"])]
        );
        t = move_project_to_folder(&t, "p1", None);
        assert_eq!(
            t.folders.iter().map(|f| f.projects.clone()).collect::<Vec<_>>(),
            vec![Vec::<String>::new(), Vec::new()]
        );
        assert_eq!(t.root_order, s(&["p1"]));
    }

    #[test]
    fn delete_folder_returns_its_projects_to_the_top_and_drops_its_collapsed_flag() {
        let (mut tree, ids) = with_folders(&["Work"]);
        tree.root_order = s(&["p9"]);
        let mut t = move_project_to_folder(&tree, "p1", Some(&ids[0]));
        t = set_collapsed(&t, &folder_node_key(&ids[0]), true);
        t = delete_folder(&t, &ids[0]);
        assert!(t.folders.is_empty());
        assert_eq!(t.root_order, s(&["p1", "p9"]));
        assert!(t.collapsed.is_empty());
    }

    #[test]
    fn move_folder_swaps_with_the_neighbor_and_ignores_moves_past_the_ends() {
        let (tree, ids) = with_folders(&["A", "B", "C"]);
        let names = |t: &TreePrefs| t.folders.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&move_folder(&tree, &ids[2], -1)), s(&["A", "C", "B"]));
        assert_eq!(move_folder(&tree, &ids[0], -1), tree);
    }

    #[test]
    fn move_project_swaps_displayed_neighbors_and_keeps_hidden_projects_in_place() {
        let tree = TreePrefs {
            root_order: s(&["a", "x"]),
            ..default_tree_prefs()
        };
        let t = move_project(&tree, "c", -1, &s(&["a", "b", "c"]));
        assert_eq!(t.root_order, s(&["a", "x", "c", "b"]));
        assert_eq!(
            arrange_projects(&t, &s(&["a", "b", "c"])).root,
            s(&["a", "c", "b"])
        );
        assert_eq!(move_project(&tree, "a", -1, &s(&["a", "b"])), tree);
    }

    #[test]
    fn move_session_swaps_displayed_neighbors_and_leaves_other_projects_untouched() {
        let tree = TreePrefs {
            session_order: orders(&[("p1", &["a", "x"]), ("p2", &["z"])]),
            ..default_tree_prefs()
        };
        let t = move_session(&tree, "p1", "c", -1, &s(&["a", "b", "c"]));
        assert_eq!(
            t.session_order,
            orders(&[("p1", &["a", "x", "c", "b"]), ("p2", &["z"])])
        );
        assert_eq!(move_session(&tree, "p1", "a", -1, &s(&["a", "b"])), tree);
    }

    #[test]
    fn prepend_session_puts_a_new_session_first_moving_it_up_from_an_earlier_position() {
        let tree = TreePrefs {
            session_order: orders(&[("p1", &["a", "b"]), ("p2", &["z"])]),
            ..default_tree_prefs()
        };
        let t = prepend_session(&tree, "p1", "c");
        assert_eq!(
            t.session_order,
            orders(&[("p1", &["c", "a", "b"]), ("p2", &["z"])])
        );
        let t2 = prepend_session(&t, "p1", "b");
        assert_eq!(
            t2.session_order,
            orders(&[("p1", &["b", "c", "a"]), ("p2", &["z"])])
        );
        assert_eq!(
            prepend_session(&tree, "p3", "x").session_order,
            orders(&[("p1", &["a", "b"]), ("p2", &["z"]), ("p3", &["x"])])
        );
    }

    #[test]
    fn rename_session_id_keeps_the_old_id_right_after_the_new_one() {
        let tree = TreePrefs {
            session_order: orders(&[("p1", &["a", "b"]), ("p2", &["z"])]),
            ..default_tree_prefs()
        };
        let t = rename_session_id(&tree, "p1", "a", "a2");
        assert_eq!(
            t.session_order,
            orders(&[("p1", &["a2", "a", "b"]), ("p2", &["z"])])
        );
        let t2 = rename_session_id(&t, "p1", "a2", "a3");
        assert_eq!(
            t2.session_order,
            orders(&[("p1", &["a3", "a2", "a", "b"]), ("p2", &["z"])])
        );
        assert_eq!(rename_session_id(&tree, "p1", "x", "y"), tree);
        assert_eq!(rename_session_id(&tree, "p1", "a", "a"), tree);
    }

    #[test]
    fn freeze_session_order_locks_in_the_displayed_order_once() {
        let tree = TreePrefs {
            session_order: orders(&[("p1", &["a", "b"])]),
            ..default_tree_prefs()
        };
        let displayed = vec![
            ("p1".to_string(), s(&["a", "b", "c"])),
            ("p2".to_string(), s(&["y", "x"])),
        ];
        let t = freeze_session_order(&tree, &displayed);
        assert_eq!(
            t.session_order,
            orders(&[("p1", &["a", "b", "c"]), ("p2", &["y", "x"])])
        );
        assert_eq!(freeze_session_order(&t, &displayed), t);
    }

    #[test]
    fn arrange_by_manual_order_keeps_stored_positions_and_appends_unlisted_ids() {
        assert_eq!(
            arrange_by_manual_order(&s(&["a", "b", "c"]), &s(&["c", "a"])),
            s(&["c", "a", "b"])
        );
        assert_eq!(
            arrange_by_manual_order(&s(&["a", "b"]), &s(&["z", "a"])),
            s(&["a", "b"])
        );
    }

    #[test]
    fn arrange_projects_places_folder_members_then_ordered_top_level_then_the_rest() {
        let (mut tree, ids) = with_folders(&["Work", "Empty"]);
        tree.root_order = s(&["z"]);
        let t = move_project_to_folder(&tree, "m", Some(&ids[0]));
        let arranged = arrange_projects(&t, &s(&["q", "m", "z", "r"]));
        let folders: Vec<_> = arranged
            .folders
            .iter()
            .map(|f| (f.folder.name.clone(), f.projects.clone()))
            .collect();
        assert_eq!(
            folders,
            vec![("Work".to_string(), s(&["m"])), ("Empty".to_string(), vec![])]
        );
        assert_eq!(arranged.root, s(&["z", "q", "r"]));
    }

    #[derive(Clone)]
    struct Item {
        id: &'static str,
        mtime: i64,
        pin: Option<SessionPin>,
        category: SessionCategory,
    }

    #[test]
    fn sort_sessions_puts_pinned_bands_around_the_sorted_rest() {
        let item = |id, mtime, pin, category| Item {
            id,
            mtime,
            pin,
            category,
        };
        let items = vec![
            item("idle-new", 9, None, SessionCategory::Idle),
            item("running", 5, None, SessionCategory::Running),
            item(
                "pin-bottom",
                8,
                Some(SessionPin::Bottom),
                SessionCategory::Waiting,
            ),
            item("waiting", 1, None, SessionCategory::Waiting),
            item("pin-top", 2, Some(SessionPin::Top), SessionCategory::Stopped),
            item("error", 3, None, SessionCategory::Error),
        ];
        let order = |sort| -> Vec<&'static str> {
            sort_sessions(&items, sort, |i| i.pin, |i| i.mtime, |i| i.category)
                .iter()
                .map(|i| i.id)
                .collect()
        };
        assert_eq!(
            order(SessionSort::Recent),
            vec!["pin-top", "idle-new", "running", "error", "waiting", "pin-bottom"]
        );
        assert_eq!(
            order(SessionSort::Actionable),
            vec!["pin-top", "error", "waiting", "running", "idle-new", "pin-bottom"]
        );
    }
}
