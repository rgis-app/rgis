//! Widget tree helpers based on parent ids.

use std::collections::{HashMap, VecDeque};

use crate::types::{Rect, WidgetRegistryEntry};

#[derive(Clone)]
struct IndexedWidget {
    key: String,
    parent_key: Option<String>,
    widget: WidgetRegistryEntry,
}

/// Collect a widget and all descendant widgets from a flat registry snapshot.
pub fn collect_subtree(
    widgets: &[WidgetRegistryEntry],
    root: &WidgetRegistryEntry,
) -> Vec<WidgetRegistryEntry> {
    let indexed = index_widgets(widgets);
    let children_map = build_children_map(&indexed);
    let mut result = Vec::new();
    let mut queue = VecDeque::new();
    let Some(root) = find_indexed_widget(&indexed, root) else {
        return result;
    };
    result.push(root.widget.clone());
    queue.push_back(root.key.clone());
    while let Some(parent_key) = queue.pop_front() {
        let children = children_map
            .get(&Some(parent_key))
            .cloned()
            .unwrap_or_default();
        for child in children {
            queue.push_back(child.key.clone());
            result.push(child.widget);
        }
    }
    result
}

fn build_children_map(indexed: &[IndexedWidget]) -> HashMap<Option<String>, Vec<IndexedWidget>> {
    let mut map: HashMap<Option<String>, Vec<IndexedWidget>> = HashMap::new();
    for widget in indexed {
        map.entry(widget.parent_key.clone())
            .or_default()
            .push(widget.clone());
    }
    map
}

fn index_widgets(widgets: &[WidgetRegistryEntry]) -> Vec<IndexedWidget> {
    let mut indexed = Vec::with_capacity(widgets.len());
    let mut sibling_counts: HashMap<(Option<String>, String), usize> = HashMap::new();

    for widget in widgets {
        let parent_key = resolve_parent_key(&indexed, widget);
        let ordinal = sibling_counts
            .entry((parent_key.clone(), widget.id.clone()))
            .or_default();
        *ordinal += 1;
        let key = scoped_key(parent_key.as_deref(), &widget.id, *ordinal);
        indexed.push(IndexedWidget {
            key: key.clone(),
            parent_key: parent_key.clone(),
            widget: widget.clone(),
        });
    }
    indexed
}

fn scoped_key(parent_key: Option<&str>, id: &str, ordinal: usize) -> String {
    match parent_key {
        Some(parent_key) => format!("{parent_key}/{id}#{ordinal}"),
        None => format!("{id}#{ordinal}"),
    }
}

fn resolve_parent_key(indexed: &[IndexedWidget], widget: &WidgetRegistryEntry) -> Option<String> {
    let parent_id = widget.parent_id.as_deref()?;
    indexed
        .iter()
        .rfind(|candidate| {
            candidate.widget.id == parent_id && rect_contains(candidate.widget.rect, widget.rect)
        })
        .or_else(|| {
            indexed
                .iter()
                .rfind(|candidate| candidate.widget.id == parent_id)
        })
        .map(|candidate| candidate.key.clone())
}

fn find_indexed_widget<'a>(
    indexed: &'a [IndexedWidget],
    root: &WidgetRegistryEntry,
) -> Option<&'a IndexedWidget> {
    indexed
        .iter()
        .find(|entry| same_widget(&entry.widget, root))
}

fn same_widget(left: &WidgetRegistryEntry, right: &WidgetRegistryEntry) -> bool {
    left.native_id == right.native_id
        && left.id == right.id
        && left.parent_id == right.parent_id
        && left.viewport_id == right.viewport_id
}

fn rect_contains(parent: Rect, child: Rect) -> bool {
    parent.min.x <= child.min.x
        && parent.min.y <= child.min.y
        && parent.max.x >= child.max.x
        && parent.max.y >= child.max.y
}
